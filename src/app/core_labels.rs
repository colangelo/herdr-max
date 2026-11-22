//! `herdr.seen` and `herdr.pinned` shared with the Herdr apps through
//! herdr-core terminal labels (openspec/changes/core-phase4, D7 to D11;
//! herdr-app `docs/labels.md`). Pure: the facts come from core's events, herdr's
//! side comes through [`HerdrView`], and the result is label writes for core
//! and effects on herdr's panes.

use std::collections::{HashMap, HashSet};

pub(crate) use crate::pane::{CoreAddress, CoreFact, LabelWrite, Observed};
use crate::terminal::TerminalId;
use crate::workspace::Workspace;

/// What herdr knows about the pane showing a core terminal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PaneFacts {
    pub(crate) seen: bool,
    /// An agent whose state is idle: the only pane a mark unread can make
    /// `done`.
    pub(crate) idle_agent: bool,
    pub(crate) pinned: bool,
}

/// herdr's panes by core address.
pub(crate) trait HerdrView {
    fn pane(&self, address: &str) -> Option<PaneFacts>;
}

impl HerdrView for HashMap<CoreAddress, PaneFacts> {
    fn pane(&self, address: &str) -> Option<PaneFacts> {
        self.get(address).copied()
    }
}

/// A change to herdr's pane that another client's label asks for (D9, D10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PaneEffect {
    MarkSeen(CoreAddress),
    MarkUnseen(CoreAddress),
    Pin(CoreAddress),
    Unpin(CoreAddress),
}

/// What one fact or look gives: writes for core, effects for herdr.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub(crate) writes: Vec<LabelWrite>,
    pub(crate) effects: Vec<PaneEffect>,
}

impl Outcome {
    fn write(&mut self, write: Option<LabelWrite>) {
        self.writes.extend(write);
    }
}

#[derive(Debug, Default)]
struct Shared {
    observed: Observed,
    /// The turn decided on while the pane stayed on screen (D8).
    decided: Option<u64>,
    /// The pin both sides last agreed on; `None` until herdr's pane was
    /// known, when the connect rule decides it (D10).
    pin_agreed: Option<bool>,
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
    pub(crate) fn apply(&mut self, fact: CoreFact, herdr: &impl HerdrView) -> Outcome {
        let mut out = Outcome::default();
        match fact {
            CoreFact::Snapshot(terminals) => {
                self.terminals = terminals
                    .into_iter()
                    .map(|(address, observed)| {
                        let shared = Shared {
                            observed,
                            ..Shared::default()
                        };
                        (address, shared)
                    })
                    .collect();
                let addresses: Vec<_> = self.terminals.keys().cloned().collect();
                for address in &addresses {
                    self.connect(address, herdr, &mut out);
                }
                out.writes.extend(self.raise_marked_early());
            }
            CoreFact::Upsert { address, observed } => {
                self.terminals.insert(
                    address.clone(),
                    Shared {
                        observed,
                        ..Shared::default()
                    },
                );
                self.connect(&address, herdr, &mut out);
                out.writes.extend(self.raise_marked_early());
            }
            CoreFact::Turn { address, turn } => {
                let Some(shared) = self.terminals.get_mut(&address) else {
                    return out;
                };
                shared.observed.turn = turn;
                if self.on_screen.contains(&address) {
                    out.write(self.decide(&address));
                }
            }
            CoreFact::Labels {
                address,
                seen,
                pinned,
            } => self.labels_changed(&address, seen, pinned, herdr, &mut out),
            CoreFact::Gone { address } => {
                self.terminals.remove(&address);
            }
            CoreFact::Disconnected => self.terminals.clear(),
        }
        out
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

    /// After herdr's state may have changed: a pin made or removed in herdr
    /// goes to the label, and a pane herdr did not know at connect gets the
    /// connect rule (D10).
    pub(crate) fn sync_pins(&mut self, herdr: &impl HerdrView) -> Outcome {
        let mut out = Outcome::default();
        let addresses: Vec<_> = self.terminals.keys().cloned().collect();
        for address in &addresses {
            let Some(pane) = herdr.pane(address) else {
                continue;
            };
            let Some(shared) = self.terminals.get_mut(address) else {
                continue;
            };
            match shared.pin_agreed {
                None => self.connect(address, herdr, &mut out),
                Some(agreed) if agreed != pane.pinned => {
                    shared.pin_agreed = Some(pane.pinned);
                    shared.observed.pinned = pane.pinned;
                    out.writes.push(LabelWrite::Pinned {
                        address: address.clone(),
                        pinned: pane.pinned,
                    });
                }
                Some(_) => {}
            }
        }
        out
    }

    /// The connect rules (D9, D10): a label at or above the turn marks the
    /// pane seen, and a pin on either side ends on both.
    fn connect(&mut self, address: &CoreAddress, herdr: &impl HerdrView, out: &mut Outcome) {
        let Some(shared) = self.terminals.get_mut(address) else {
            return;
        };
        if self.on_screen.contains(address) {
            out.write(decide_shared(address, shared));
        }
        let Some(pane) = herdr.pane(address) else {
            return;
        };
        let observed = shared.observed;
        if !pane.seen && observed.seen.is_some_and(|seen| seen >= observed.turn) {
            out.effects.push(PaneEffect::MarkSeen(address.clone()));
        }
        match (pane.pinned, observed.pinned) {
            (true, false) => {
                shared.observed.pinned = true;
                out.writes.push(LabelWrite::Pinned {
                    address: address.clone(),
                    pinned: true,
                });
            }
            (false, true) => out.effects.push(PaneEffect::Pin(address.clone())),
            _ => {}
        }
        shared.pin_agreed = Some(pane.pinned || observed.pinned);
    }

    /// Another client changed the labels (D9, D10). herdr's own writes come
    /// back here too; they match what herdr already shows, so they change
    /// nothing.
    fn labels_changed(
        &mut self,
        address: &CoreAddress,
        seen: Option<u64>,
        pinned: bool,
        herdr: &impl HerdrView,
        out: &mut Outcome,
    ) {
        let Some(shared) = self.terminals.get_mut(address) else {
            return;
        };
        let turn = shared.observed.turn;
        let was = shared.observed.seen;
        shared.observed.seen = seen;
        shared.observed.pinned = pinned;
        let Some(pane) = herdr.pane(address) else {
            return;
        };
        let read = |label: Option<u64>| label.is_some_and(|label| label >= turn);
        if read(seen) && !read(was) && !pane.seen {
            out.effects.push(PaneEffect::MarkSeen(address.clone()));
        }
        if seen.is_some() && !read(seen) && read(was) && pane.seen && pane.idle_agent {
            out.effects.push(PaneEffect::MarkUnseen(address.clone()));
        }
        if shared.pin_agreed.is_some_and(|agreed| agreed != pinned) {
            shared.pin_agreed = Some(pinned);
            if pane.pinned != pinned {
                out.effects.push(if pinned {
                    PaneEffect::Pin(address.clone())
                } else {
                    PaneEffect::Unpin(address.clone())
                });
            }
        }
    }

    fn decide(&mut self, address: &CoreAddress) -> Option<LabelWrite> {
        decide_shared(address, self.terminals.get_mut(address)?)
    }
}

/// Once per turn while on screen: raise `herdr.seen` to the turn when it is
/// missing or lower. A label another client lowered meanwhile (mark unread)
/// is left until the next turn or the next look.
fn decide_shared(address: &str, shared: &mut Shared) -> Option<LabelWrite> {
    let turn = shared.observed.turn;
    if shared.decided == Some(turn) {
        return None;
    }
    shared.decided = Some(turn);
    raise(address, &mut shared.observed)
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
        Observed {
            turn,
            seen,
            pinned: false,
        }
    }

    /// No pane herdr knows: label writes only, no effects.
    fn nobody() -> HashMap<CoreAddress, PaneFacts> {
        HashMap::new()
    }

    fn known(terminals: &[(&str, Observed)]) -> CoreLabels {
        let mut labels = CoreLabels::default();
        let snapshot = terminals
            .iter()
            .map(|(address, observed)| (address.to_string(), *observed))
            .collect();
        assert_eq!(
            labels.apply(CoreFact::Snapshot(snapshot), &nobody()),
            Outcome::default()
        );
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
        labels.apply(
            CoreFact::Labels {
                address: "t1".into(),
                seen: Some(2),
                pinned: false,
            },
            &nobody(),
        );
        assert_eq!(labels.look(screen(&["t1"])), vec![], "still on screen");

        assert_eq!(labels.look(screen(&[])), vec![]);
        assert_eq!(
            labels.look(screen(&["t1"])),
            vec![seen("t1", 3)],
            "looked again"
        );

        labels.apply(
            CoreFact::Labels {
                address: "t1".into(),
                seen: Some(2),
                pinned: false,
            },
            &nobody(),
        );
        assert_eq!(
            labels
                .apply(
                    CoreFact::Turn {
                        address: "t1".into(),
                        turn: 4
                    },
                    &nobody()
                )
                .writes,
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
            labels
                .apply(
                    CoreFact::Turn {
                        address: "t1".into(),
                        turn: 4
                    },
                    &nobody()
                )
                .writes,
            vec![seen("t1", 4)]
        );
        assert_eq!(
            labels
                .apply(
                    CoreFact::Turn {
                        address: "t2".into(),
                        turn: 4
                    },
                    &nobody()
                )
                .writes,
            vec![]
        );
    }

    #[test]
    fn a_snapshot_after_a_reconnect_decides_the_panes_on_screen() {
        let mut labels = known(&[("t1", at(2, Some(2)))]);
        labels.look(screen(&["t1"]));
        labels.apply(CoreFact::Disconnected, &nobody());
        assert_eq!(
            labels
                .apply(
                    CoreFact::Snapshot(vec![("t1".into(), at(3, Some(2)))]),
                    &nobody()
                )
                .writes,
            vec![seen("t1", 3)]
        );
        assert_eq!(
            labels
                .apply(
                    CoreFact::Upsert {
                        address: "t9".into(),
                        observed: at(1, None)
                    },
                    &nobody()
                )
                .writes,
            vec![],
            "not on screen"
        );
        assert_eq!(
            labels
                .apply(
                    CoreFact::Gone {
                        address: "t1".into()
                    },
                    &nobody()
                )
                .writes,
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
            labels
                .apply(
                    CoreFact::Snapshot(vec![("t1".into(), at(2, Some(0)))]),
                    &nobody()
                )
                .writes,
            vec![seen("t1", 2)]
        );
        assert_eq!(
            labels
                .apply(
                    CoreFact::Snapshot(vec![("t1".into(), at(3, Some(2)))]),
                    &nobody()
                )
                .writes,
            vec![],
            "raised once"
        );
    }

    fn panes(entries: &[(&str, PaneFacts)]) -> HashMap<CoreAddress, PaneFacts> {
        entries
            .iter()
            .map(|(address, facts)| (address.to_string(), *facts))
            .collect()
    }

    const DONE: PaneFacts = PaneFacts {
        seen: false,
        idle_agent: true,
        pinned: false,
    };
    const READ: PaneFacts = PaneFacts {
        seen: true,
        idle_agent: true,
        pinned: false,
    };

    fn label(address: &str, seen: Option<u64>, pinned: bool) -> CoreFact {
        CoreFact::Labels {
            address: address.into(),
            seen,
            pinned,
        }
    }

    fn snapshot_of(
        labels: &mut CoreLabels,
        entries: &[(&str, Observed)],
        herdr: &HashMap<CoreAddress, PaneFacts>,
    ) -> Outcome {
        let snapshot = entries
            .iter()
            .map(|(address, observed)| (address.to_string(), *observed))
            .collect();
        labels.apply(CoreFact::Snapshot(snapshot), herdr)
    }

    // core-phase4 verifier 11: a label raised to the turn marks a done pane
    // seen; a stale raise does nothing.
    #[test]
    fn a_raise_to_the_turn_marks_the_pane_seen() {
        let herdr = panes(&[("t1", DONE)]);
        let mut labels = CoreLabels::default();
        assert_eq!(
            snapshot_of(&mut labels, &[("t1", at(4, Some(2)))], &herdr),
            Outcome::default()
        );
        assert_eq!(
            labels.apply(label("t1", Some(3), false), &herdr),
            Outcome::default(),
            "stale"
        );
        assert_eq!(
            labels.apply(label("t1", Some(4), false), &herdr).effects,
            vec![PaneEffect::MarkSeen("t1".into())]
        );
        assert_eq!(
            labels.apply(label("t1", Some(4), false), &panes(&[("t1", READ)])),
            Outcome::default(),
            "herdr's own write coming back changes nothing"
        );
    }

    // core-phase4 verifier 12: a lowered label marks an idle, seen agent
    // unseen; a working agent, a non-agent and a removed label do not move.
    #[test]
    fn a_mark_unread_makes_an_idle_agent_done() {
        let mut labels = CoreLabels::default();
        snapshot_of(
            &mut labels,
            &[("t1", at(3, Some(3)))],
            &panes(&[("t1", READ)]),
        );
        assert_eq!(
            labels
                .apply(label("t1", Some(2), false), &panes(&[("t1", READ)]))
                .effects,
            vec![PaneEffect::MarkUnseen("t1".into())]
        );
        let working = PaneFacts {
            idle_agent: false,
            ..READ
        };
        for (herdr, seen) in [(working, Some(2)), (READ, None)] {
            let mut labels = CoreLabels::default();
            snapshot_of(
                &mut labels,
                &[("t1", at(3, Some(3)))],
                &panes(&[("t1", herdr)]),
            );
            assert!(
                labels
                    .apply(label("t1", seen, false), &panes(&[("t1", herdr)]))
                    .effects
                    .is_empty(),
                "{herdr:?} {seen:?}"
            );
        }
    }

    #[test]
    fn at_connect_a_read_label_marks_the_pane_seen_and_never_unseen() {
        let mut labels = CoreLabels::default();
        let herdr = panes(&[("t1", DONE), ("t2", READ)]);
        let out = snapshot_of(
            &mut labels,
            &[("t1", at(5, Some(5))), ("t2", at(5, Some(1)))],
            &herdr,
        );
        assert_eq!(out.effects, vec![PaneEffect::MarkSeen("t1".into())]);
    }

    // core-phase4 verifier 14: pins both ways, and the connect union.
    #[test]
    fn pins_travel_both_ways_and_meet_at_connect() {
        let pinned = PaneFacts {
            pinned: true,
            ..READ
        };
        let mut labels = CoreLabels::default();
        let mut out = snapshot_of(
            &mut labels,
            &[
                ("t1", at(1, Some(1))),
                (
                    "t2",
                    Observed {
                        pinned: true,
                        ..at(1, Some(1))
                    },
                ),
            ],
            &panes(&[("t1", pinned), ("t2", READ)]),
        );
        out.writes.sort_by_key(|write| format!("{write:?}"));
        assert_eq!(
            out.writes,
            vec![LabelWrite::Pinned {
                address: "t1".into(),
                pinned: true
            }]
        );
        assert_eq!(out.effects, vec![PaneEffect::Pin("t2".into())]);

        let herdr = panes(&[("t1", pinned), ("t2", pinned)]);
        assert_eq!(labels.sync_pins(&herdr), Outcome::default(), "both agree");
        let herdr = panes(&[("t1", READ), ("t2", pinned)]);
        assert_eq!(
            labels.sync_pins(&herdr).writes,
            vec![LabelWrite::Pinned {
                address: "t1".into(),
                pinned: false
            }],
            "unpinned in herdr"
        );
        assert_eq!(
            labels.apply(label("t1", Some(1), false), &herdr),
            Outcome::default(),
            "the echo of herdr's unpin"
        );
        assert_eq!(
            labels.apply(label("t2", Some(1), false), &herdr).effects,
            vec![PaneEffect::Unpin("t2".into())],
            "unpinned on the phone"
        );
        assert_eq!(
            labels
                .apply(
                    label("t2", Some(1), true),
                    &panes(&[("t1", READ), ("t2", READ)])
                )
                .effects,
            vec![PaneEffect::Pin("t2".into())],
            "pinned on the phone"
        );
    }

    #[test]
    fn a_pane_herdr_learns_later_gets_the_connect_rule() {
        let mut labels = CoreLabels::default();
        snapshot_of(
            &mut labels,
            &[(
                "t1",
                Observed {
                    pinned: true,
                    ..at(2, Some(2))
                },
            )],
            &nobody(),
        );
        let out = labels.sync_pins(&panes(&[("t1", DONE)]));
        assert_eq!(
            out.effects,
            vec![
                PaneEffect::MarkSeen("t1".into()),
                PaneEffect::Pin("t1".into())
            ]
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
