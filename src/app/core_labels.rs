//! `herdr.seen` shared with the Herdr apps through herdr-core terminal labels
//! (openspec/changes/core-phase4, D7, D8 and D11; herdr-app `docs/labels.md`).
//! Pure: the facts come from core's events, the writes go back to core, and
//! nothing here touches a core or a terminal.

use std::collections::{HashMap, HashSet};

pub(crate) use crate::pane::{CoreAddress, CoreFact, LabelWrite, Observed};
use crate::terminal::TerminalId;
use crate::workspace::Workspace;

#[derive(Debug, Default)]
struct Shared {
    observed: Observed,
    /// The turn decided on while the pane stayed on screen (D8).
    decided: Option<u64>,
}

/// The label state of herdr's core terminals and the panes on screen.
#[derive(Debug, Default)]
pub(crate) struct CoreLabels {
    terminals: HashMap<CoreAddress, Shared>,
    on_screen: HashSet<CoreAddress>,
    /// Marked read before core reported the terminal (the subscription was
    /// still starting); raised when it does.
    marked_early: HashSet<CoreAddress>,
}

impl CoreLabels {
    pub(crate) fn apply(&mut self, fact: CoreFact) -> Vec<LabelWrite> {
        match fact {
            CoreFact::Snapshot(terminals) => {
                self.terminals = terminals
                    .into_iter()
                    .map(|(address, observed)| {
                        let shared = Shared {
                            observed,
                            decided: None,
                        };
                        (address, shared)
                    })
                    .collect();
                let on_screen: Vec<_> = self.on_screen.iter().cloned().collect();
                let mut writes: Vec<_> = on_screen
                    .iter()
                    .filter_map(|address| self.decide(address))
                    .collect();
                writes.extend(self.raise_marked_early());
                writes
            }
            CoreFact::Upsert { address, observed } => {
                self.terminals.entry(address.clone()).or_default().observed = observed;
                let mut writes = self.decide_if_on_screen(&address);
                writes.extend(self.raise_marked_early());
                writes
            }
            CoreFact::Turn { address, turn } => {
                let Some(shared) = self.terminals.get_mut(&address) else {
                    return Vec::new();
                };
                shared.observed.turn = turn;
                self.decide_if_on_screen(&address)
            }
            CoreFact::Labels { address, seen } => {
                if let Some(shared) = self.terminals.get_mut(&address) {
                    shared.observed.seen = seen;
                }
                Vec::new()
            }
            CoreFact::Gone { address } => {
                self.terminals.remove(&address);
                Vec::new()
            }
            CoreFact::Disconnected => {
                self.terminals.clear();
                Vec::new()
            }
        }
    }

    /// The panes a person is looking at now. A pane that left the screen is
    /// decided again when it comes back.
    pub(crate) fn look(&mut self, on_screen: HashSet<CoreAddress>) -> Vec<LabelWrite> {
        for address in self.on_screen.difference(&on_screen) {
            if let Some(shared) = self.terminals.get_mut(address) {
                shared.decided = None;
            }
        }
        let arrived: Vec<_> = on_screen.difference(&self.on_screen).cloned().collect();
        self.on_screen = on_screen;
        arrived
            .iter()
            .filter_map(|address| self.decide(address))
            .collect()
    }

    /// A person marked the pane read (`pane.mark_seen`): raise the label to
    /// the turn, on screen or not.
    pub(crate) fn herdr_marked_seen(&mut self, address: &str) -> Option<LabelWrite> {
        let Some(shared) = self.terminals.get_mut(address) else {
            self.marked_early.insert(address.to_string());
            return None;
        };
        raise(address, &mut shared.observed)
    }

    fn raise_marked_early(&mut self) -> Vec<LabelWrite> {
        let known: Vec<_> = self
            .marked_early
            .iter()
            .filter(|address| self.terminals.contains_key(*address))
            .cloned()
            .collect();
        known
            .iter()
            .filter_map(|address| {
                self.marked_early.remove(address);
                self.herdr_marked_seen(address)
            })
            .collect()
    }

    fn decide_if_on_screen(&mut self, address: &CoreAddress) -> Vec<LabelWrite> {
        if !self.on_screen.contains(address) {
            return Vec::new();
        }
        self.decide(address).into_iter().collect()
    }

    /// Once per turn while on screen: raise `herdr.seen` to the turn when it
    /// is missing or lower. A label another client lowered meanwhile (mark
    /// unread) is left until the next turn or the next look.
    fn decide(&mut self, address: &CoreAddress) -> Option<LabelWrite> {
        let shared = self.terminals.get_mut(address)?;
        let turn = shared.observed.turn;
        if shared.decided == Some(turn) {
            return None;
        }
        shared.decided = Some(turn);
        raise(address, &mut shared.observed)
    }
}

fn raise(address: &str, observed: &mut Observed) -> Option<LabelWrite> {
    let turn = observed.turn;
    if observed.seen.is_some_and(|seen| seen >= turn) {
        return None;
    }
    observed.seen = Some(turn);
    Some(LabelWrite::Seen {
        address: address.to_string(),
        turn,
    })
}

/// One attached shell client's view: whether its outer terminal reported
/// focus-in, and the tab it shows.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ClientView {
    pub(crate) focused: Option<bool>,
    pub(crate) workspace_index: usize,
    pub(crate) tab_index: usize,
}

/// The core panes some person is looking at (D7): drawn in the tab of a
/// client whose terminal has focus, and not hidden by a zoomed pane. A
/// terminal that never reported focus does not count.
pub(crate) fn panes_on_screen(
    views: impl IntoIterator<Item = ClientView>,
    workspaces: &[Workspace],
    core_address: impl Fn(&TerminalId) -> Option<CoreAddress>,
) -> HashSet<CoreAddress> {
    let mut on_screen = HashSet::new();
    for view in views {
        if view.focused != Some(true) {
            continue;
        }
        let Some(tab) = workspaces
            .get(view.workspace_index)
            .and_then(|ws| ws.tabs.get(view.tab_index))
        else {
            continue;
        };
        let zoomed = tab.zoomed.then(|| tab.layout.focused());
        on_screen.extend(
            tab.panes
                .iter()
                .filter(|(pane_id, _)| zoomed.is_none_or(|zoomed| zoomed == **pane_id))
                .filter_map(|(_, pane)| core_address(&pane.attached_terminal_id)),
        );
    }
    on_screen
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(turn: u64, seen: Option<u64>) -> Observed {
        Observed { turn, seen }
    }

    fn known(terminals: &[(&str, Observed)]) -> CoreLabels {
        let mut labels = CoreLabels::default();
        let snapshot = terminals
            .iter()
            .map(|(address, observed)| (address.to_string(), *observed))
            .collect();
        assert!(labels.apply(CoreFact::Snapshot(snapshot)).is_empty());
        labels
    }

    fn screen(addresses: &[&str]) -> HashSet<CoreAddress> {
        addresses.iter().map(|a| a.to_string()).collect()
    }

    fn seen(address: &str, turn: u64) -> LabelWrite {
        LabelWrite::Seen {
            address: address.into(),
            turn,
        }
    }

    // core-phase4 verifier 5: looking raises a lower or missing label to
    // the turn, and never lowers one.
    #[test]
    fn looking_raises_the_label_and_never_lowers_it() {
        let mut labels = known(&[
            ("t1", at(3, Some(1))),
            ("t2", at(3, None)),
            ("t3", at(3, Some(3))),
            ("t4", at(3, Some(5))),
        ]);
        let mut writes = labels.look(screen(&["t1", "t2", "t3", "t4"]));
        writes.sort_by_key(|write| format!("{write:?}"));
        assert_eq!(writes, vec![seen("t1", 3), seen("t2", 3)]);
        assert_eq!(labels.look(screen(&["t1", "t2", "t3", "t4"])), vec![]);
    }

    // core-phase4 verifier 6: one decision per turn while on screen, so a
    // mark unread from another client stands until the pane leaves and
    // returns, or the next turn.
    #[test]
    fn a_mark_unread_stands_while_the_pane_stays_on_screen() {
        let mut labels = known(&[("t1", at(3, Some(3)))]);
        assert_eq!(labels.look(screen(&["t1"])), vec![]);
        labels.apply(CoreFact::Labels {
            address: "t1".into(),
            seen: Some(2),
        });
        assert_eq!(labels.look(screen(&["t1"])), vec![], "still on screen");

        assert_eq!(labels.look(screen(&[])), vec![]);
        assert_eq!(
            labels.look(screen(&["t1"])),
            vec![seen("t1", 3)],
            "looked again"
        );

        labels.apply(CoreFact::Labels {
            address: "t1".into(),
            seen: Some(2),
        });
        assert_eq!(
            labels.apply(CoreFact::Turn {
                address: "t1".into(),
                turn: 4
            }),
            vec![seen("t1", 4)],
            "a new turn is decided"
        );
    }

    // core-phase4 verifier 7: the turn event decides for a pane on screen,
    // and only for one on screen.
    #[test]
    fn the_turn_event_decides_for_panes_on_screen() {
        let mut labels = known(&[("t1", at(3, Some(3))), ("t2", at(3, Some(3)))]);
        labels.look(screen(&["t1"]));
        assert_eq!(
            labels.apply(CoreFact::Turn {
                address: "t1".into(),
                turn: 4
            }),
            vec![seen("t1", 4)]
        );
        assert_eq!(
            labels.apply(CoreFact::Turn {
                address: "t2".into(),
                turn: 4
            }),
            vec![]
        );
    }

    #[test]
    fn a_snapshot_after_a_reconnect_decides_the_panes_on_screen() {
        let mut labels = known(&[("t1", at(2, Some(2)))]);
        labels.look(screen(&["t1"]));
        labels.apply(CoreFact::Disconnected);
        assert_eq!(
            labels.apply(CoreFact::Snapshot(vec![("t1".into(), at(3, Some(2)))])),
            vec![seen("t1", 3)]
        );
        assert_eq!(
            labels.apply(CoreFact::Upsert {
                address: "t9".into(),
                observed: at(1, None)
            }),
            vec![],
            "not on screen"
        );
        assert_eq!(
            labels.apply(CoreFact::Gone {
                address: "t1".into()
            }),
            vec![]
        );
        assert_eq!(labels.herdr_marked_seen("t1"), None, "gone");
    }

    // core-phase4 verifier 9 (reducer half): an explicit mark read raises
    // the label off screen too, once.
    #[test]
    fn mark_seen_raises_the_label_off_screen() {
        let mut labels = known(&[("t1", at(4, Some(2))), ("t2", at(4, Some(4)))]);
        assert_eq!(labels.herdr_marked_seen("t1"), Some(seen("t1", 4)));
        assert_eq!(labels.herdr_marked_seen("t1"), None);
        assert_eq!(labels.herdr_marked_seen("t2"), None);
        assert_eq!(labels.herdr_marked_seen("t404"), None);
    }

    #[test]
    fn a_mark_read_before_core_reports_the_terminal_lands_when_it_does() {
        let mut labels = CoreLabels::default();
        assert_eq!(labels.herdr_marked_seen("t1"), None, "not known yet");
        assert_eq!(
            labels.apply(CoreFact::Snapshot(vec![("t1".into(), at(2, Some(0)))])),
            vec![seen("t1", 2)]
        );
        assert_eq!(
            labels.apply(CoreFact::Snapshot(vec![("t1".into(), at(3, Some(2)))])),
            vec![],
            "raised once"
        );
    }

    fn two_pane_space() -> (Vec<Workspace>, Vec<TerminalId>) {
        let mut ws = Workspace::test_new("w");
        let first = ws.tabs[0].root_pane;
        let second = ws.test_split(ratatui::layout::Direction::Horizontal);
        let ids = [first, second]
            .iter()
            .map(|pane| ws.tabs[0].panes[pane].attached_terminal_id.clone())
            .collect();
        (vec![ws], ids)
    }

    fn view(focused: Option<bool>) -> ClientView {
        ClientView {
            focused,
            workspace_index: 0,
            tab_index: 0,
        }
    }

    // core-phase4 verifier 8: what counts as on screen.
    #[test]
    fn only_a_focused_client_puts_its_drawn_panes_on_screen() {
        let (mut workspaces, ids) = two_pane_space();
        let address = |id: &TerminalId| {
            ids.iter()
                .position(|known| known == id)
                .map(|n| format!("t{}", n + 1))
        };
        assert_eq!(
            panes_on_screen([view(Some(true))], &workspaces, address),
            screen(&["t1", "t2"])
        );
        for focus in [Some(false), None] {
            assert!(
                panes_on_screen([view(focus)], &workspaces, address).is_empty(),
                "{focus:?}"
            );
        }
        let out_of_range = ClientView {
            focused: Some(true),
            workspace_index: 0,
            tab_index: 7,
        };
        assert!(panes_on_screen([out_of_range], &workspaces, address).is_empty());

        let tab = &mut workspaces[0].tabs[0];
        tab.zoomed = true;
        let focused = tab.layout.focused();
        let zoomed = address(&tab.panes[&focused].attached_terminal_id).unwrap();
        assert_eq!(
            panes_on_screen([view(Some(true))], &workspaces, address),
            screen(&[zoomed.as_str()]),
            "a zoomed pane covers the others"
        );
        assert!(
            panes_on_screen([view(Some(true))], &workspaces, |_| None).is_empty(),
            "panes not in core are not followed"
        );
    }
}
