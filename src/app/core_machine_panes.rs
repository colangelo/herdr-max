//! Panes on core machines (openspec/changes/core-phase5, D5): which machines
//! this server's panes run on, and the panes that wait for one.
//!
//! Core sends no event when a machine's link drops (P5), so while the server
//! has a pane on a machine it reads `machine.list` every `MACHINE_POLL`, on
//! a thread of its own and only while a lease allows that machine's link (a
//! dormant machine is not asked about). What it reads marks the machine for
//! the panes' input (`core_machine::panes`), and a pane that waits for a
//! machine attaches to its program once the machine is connected. A
//! restore, a live handoff and this poll never dial: they are not lease
//! holders (D3).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{info, warn};

use super::App;
use crate::core_machine::panes::{self, Probe};
use crate::core_machine::MachineName;
use crate::layout::PaneId;
use crate::terminal::state::{CoreWaitReason, CoreWaiting, PaneHold};

/// How often the server reads its machines' state (D5).
const MACHINE_POLL: Duration = Duration::from_secs(5);

/// The machine part of a core address (`ac-mbm5/t7`), `None` for this Mac.
pub(crate) fn machine_of(address: &str) -> Option<MachineName> {
    crate::pane::core_machines::machine_of_address(address)
}

/// Whether a probe is running, so a slow `machine.list` never stacks.
#[derive(Default)]
pub(crate) struct MachinePoll {
    pub at: Option<Instant>,
    running: Arc<AtomicBool>,
}

impl App {
    /// Panes waiting for a machine, with what they wait for.
    fn machine_waiting_panes(&self) -> Vec<(PaneId, MachineName, CoreWaiting)> {
        self.state
            .workspaces
            .iter()
            .flat_map(|ws| ws.tabs.iter())
            .flat_map(|tab| tab.layout.pane_ids())
            .filter_map(|pane_id| {
                let waiting = self.state.pane_terminal(pane_id)?.core_waiting()?;
                let machine = machine_of(&waiting.link.address)?;
                Some((pane_id, machine, waiting.clone()))
            })
            .collect()
    }

    /// The machines this server's panes run on or wait for.
    fn machines_with_panes(&self) -> Vec<MachineName> {
        let mut machines: Vec<MachineName> = self
            .terminal_runtimes
            .values()
            .filter_map(|runtime| runtime.core_machine().cloned())
            .chain(
                self.machine_waiting_panes()
                    .into_iter()
                    .map(|(_, machine, _)| machine),
            )
            .collect();
        machines.sort_unstable();
        machines.dedup();
        machines
    }

    pub(crate) fn core_machine_poll_deadline(&self) -> Option<Instant> {
        self.core_machine_poll.at
    }

    /// Arms the next read of the machines, while any pane is on one.
    pub(crate) fn schedule_core_machine_poll(&mut self, now: Instant) {
        if self.core_machine_poll.at.is_none() && !self.machines_with_panes().is_empty() {
            self.core_machine_poll.at = Some(now + MACHINE_POLL);
        }
    }

    /// When it is time, reads the machines on a thread of its own;
    /// `AppEvent::CoreMachines` brings the answer back.
    pub(crate) fn poll_core_machines_if_due(&mut self, now: Instant) -> bool {
        if self.core_machine_poll.at.is_none_or(|at| now < at) {
            return false;
        }
        self.core_machine_poll.at = None;
        let machines = self.machines_with_panes();
        if machines.is_empty() {
            return false;
        }
        if self.core_machine_poll.running.swap(true, Ordering::AcqRel) {
            self.core_machine_poll.at = Some(now + MACHINE_POLL);
            return false;
        }
        let events = self.event_tx.clone();
        let running = Arc::clone(&self.core_machine_poll.running);
        let spawned = std::thread::Builder::new()
            .name("herdr-machine-poll".into())
            .spawn(move || {
                let probes = panes::probe(&machines);
                running.store(false, Ordering::Release);
                let _ = events.blocking_send(crate::events::AppEvent::CoreMachines { probes });
            });
        if let Err(err) = spawned {
            self.core_machine_poll
                .running
                .store(false, Ordering::Release);
            warn!(err = %err, "could not start the core machine poll; retrying later");
            self.core_machine_poll.at = Some(now + MACHINE_POLL);
        }
        false
    }

    /// What a poll or a dialer found: marks each machine for its panes'
    /// input, and attaches or relabels the panes that wait for one.
    pub(crate) fn handle_core_machines(&mut self, probes: &[Probe], now: Instant) {
        for probe in probes.iter().filter(|probe| probe.read) {
            if panes::link(&probe.machine).set_connected(probe.connected()) {
                info!(
                    machine = %probe.machine,
                    state = probe.state.as_deref().unwrap_or("dormant"),
                    "core machine of this server's panes changed"
                );
            }
        }
        let mut changed = false;
        for (pane_id, machine, waiting) in self.machine_waiting_panes() {
            let Some(probe) = probes
                .iter()
                .find(|probe| probe.machine == machine && probe.read)
            else {
                continue;
            };
            changed |= if probe.connected() {
                self.attach_machine_pane(pane_id, &machine, &waiting)
            } else {
                self.set_machine_wait(pane_id, &machine, !probe.leased)
            };
        }
        self.schedule_core_machine_poll(now);
        if changed {
            self.state.mark_session_dirty();
            self.render_dirty.request_generic();
            self.render_notify.notify_one();
        }
    }

    fn set_machine_wait(&mut self, pane_id: PaneId, machine: &MachineName, dormant: bool) -> bool {
        let Some(terminal) = self.state.pane_terminal_mut(pane_id) else {
            return false;
        };
        let Some(PaneHold::CoreWaiting(waiting)) = terminal.hold.as_mut() else {
            return false;
        };
        let why = CoreWaitReason::MachineAway {
            machine: machine.to_string(),
            dormant,
        };
        if waiting.why == why {
            return false;
        }
        waiting.why = why;
        terminal.revision = terminal.revision.saturating_add(1);
        true
    }

    /// A waiting pane's machine is connected: attach to its program there.
    /// A program core says is gone is never started again, here or there:
    /// the pane says so and a person closes it (D5).
    fn attach_machine_pane(
        &mut self,
        pane_id: PaneId,
        machine: &MachineName,
        waiting: &CoreWaiting,
    ) -> bool {
        use crate::pane::CoreAttachFailure;
        let message = match self.attach_core_terminal(pane_id, waiting) {
            Ok(()) => return true,
            Err(CoreAttachFailure::Unreachable) => return false,
            Err(CoreAttachFailure::Gone) => format!("Its program on {machine} has ended."),
            Err(CoreAttachFailure::NotOurs) => format!(
                "Its terminal on {machine} ({}) is labelled for another pane; nothing was attached.",
                waiting.link.address
            ),
        };
        info!(pane = pane_id.raw(), %machine, address = %waiting.link.address, "{message}");
        let Some(terminal) = self.state.pane_terminal_mut(pane_id) else {
            return false;
        };
        terminal.hold = None;
        terminal.restore_error = Some(message);
        terminal.revision = terminal.revision.saturating_add(1);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane::CoreTerminalLink;

    fn name(raw: &str) -> MachineName {
        MachineName::parse(raw).expect("name")
    }

    fn app_with_one_pane() -> (App, PaneId) {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = vec![crate::workspace::Workspace::test_new("machine-panes")];
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        app.state.default_shell = "/bin/cat".into();
        let pane = app.state.workspaces[0].tabs[0].root_pane;
        (app, pane)
    }

    fn link_on(address: &str) -> CoreTerminalLink {
        CoreTerminalLink {
            agent: None,
            session: crate::pane::core_session_name(),
            address: address.into(),
            program: None,
        }
    }

    /// `pane_id` waits for its machine with a link to `address`.
    fn wait_for_machine(app: &mut App, pane_id: PaneId, address: &str, dormant: bool) {
        let public_pane_id = app.public_pane_id(0, pane_id);
        let machine = machine_of(address).expect("a machine address").to_string();
        app.state.pane_terminal_mut(pane_id).expect("terminal").hold =
            Some(PaneHold::CoreWaiting(CoreWaiting {
                link: link_on(address),
                public_pane_id,
                why: CoreWaitReason::MachineAway { machine, dormant },
            }));
    }

    fn why(app: &App, pane_id: PaneId) -> Option<CoreWaitReason> {
        app.state
            .pane_terminal(pane_id)
            .and_then(|terminal| terminal.core_waiting())
            .map(|waiting| waiting.why.clone())
    }

    // C4 (D5): a `machine.list` that failed reads nothing: the machine keeps
    // its mark and its panes their reason.
    #[test]
    fn an_unread_probe_changes_neither_the_machine_nor_its_panes() {
        let (mut app, pane) = app_with_one_pane();
        wait_for_machine(&mut app, pane, "unread/t3", true);
        let link = panes::link(&name("unread"));
        assert!(link.connected());
        let unread = Probe {
            machine: name("unread"),
            state: None,
            leased: true,
            read: false,
        };
        app.handle_core_machines(&[unread], Instant::now());
        assert!(link.connected(), "not marked away by a failed read");
        assert_eq!(
            why(&app, pane),
            Some(CoreWaitReason::MachineAway {
                machine: "unread".into(),
                dormant: true
            })
        );
    }
}
