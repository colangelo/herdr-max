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
                // An ended pane holds its link only so that nothing starts it.
                if matches!(waiting.why, CoreWaitReason::MachineEnded { .. }) {
                    return None;
                }
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
    /// Only a finished record ends it, and a program that ended is never
    /// started again, here or there: the pane keeps its link, ended, and a
    /// person closes it. A terminal core does not list yet keeps the pane
    /// waiting, asked about again on the next poll (D5).
    fn attach_machine_pane(
        &mut self,
        pane_id: PaneId,
        machine: &MachineName,
        waiting: &CoreWaiting,
    ) -> bool {
        let failure = match self.attach_core_terminal(pane_id, waiting) {
            Ok(()) => return true,
            Err(failure) => failure,
        };
        let Some(why) = panes::machine_wait_reason(machine, failure) else {
            return false;
        };
        if waiting.why == why {
            return false;
        }
        info!(pane = pane_id.raw(), %machine, address = %waiting.link.address, cause = why.as_str(), "core answered for the pane's program on its machine");
        let Some(terminal) = self.state.pane_terminal_mut(pane_id) else {
            return false;
        };
        let Some(PaneHold::CoreWaiting(held)) = terminal.hold.as_mut() else {
            return false;
        };
        held.why = why;
        terminal.revision = terminal.revision.saturating_add(1);
        true
    }

    /// The program of `pane_id`, on a core machine, lost its stream with a
    /// suspend cause (a stop of this Mac's core, a far core's restart): the
    /// pane waits for its machine with its link, as after a restart, and
    /// attaches once core has the machine connected again. A suspension
    /// would start it again here (D5). False for a pane on this Mac.
    pub(super) fn wait_for_machine_after_exit(&mut self, pane_id: PaneId) -> bool {
        let Some((ws_idx, pane)) = self.find_pane(pane_id) else {
            return false;
        };
        let terminal_id = pane.attached_terminal_id.clone();
        let Some(link) = self
            .terminal_runtimes
            .get(&terminal_id)
            .and_then(crate::terminal::TerminalRuntime::core_terminal)
        else {
            return false;
        };
        let Some(machine) = machine_of(&link.address) else {
            return false;
        };
        let public_pane_id = self.public_pane_id(ws_idx, pane_id);
        // The program the pane now waits for may still run there: the
        // runtime only detaches, it never ends it (D5).
        if let Some(runtime) = self.terminal_runtimes.remove(&terminal_id) {
            runtime.set_keep_core_terminal(true);
            runtime.shutdown();
        }
        panes::link(&machine).set_connected(false);
        let Some(terminal) = self.state.terminals.get_mut(&terminal_id) else {
            return false;
        };
        info!(pane = pane_id.raw(), %machine, address = %link.address, "the pane's stream ended; it waits for its machine");
        terminal.hold = Some(PaneHold::CoreWaiting(CoreWaiting {
            link,
            public_pane_id,
            why: CoreWaitReason::MachineAway {
                machine: machine.to_string(),
                dormant: false,
            },
        }));
        terminal.revision = terminal.revision.saturating_add(1);
        self.state.mark_session_dirty();
        self.schedule_core_machine_poll(Instant::now());
        self.render_dirty.request_generic();
        self.render_notify.notify_one();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::{Method, PaneRespawnParams, Request};
    use crate::pane::{CoreTerminalLink, FakeCore};

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

    /// The launch label of the runs these tests' links name.
    const LAUNCH: &str = "L1";

    fn link_on(address: &str) -> CoreTerminalLink {
        CoreTerminalLink {
            agent: None,
            session: crate::pane::core_session_name(),
            address: address.into(),
            program: None,
            launch: Some(LAUNCH.into()),
            machine_cwd: None,
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

    /// The folder on the machine a waiting pane's link names.
    fn set_machine_cwd(app: &mut App, pane_id: PaneId, cwd: &std::path::Path) {
        let terminal = app.state.pane_terminal_mut(pane_id).expect("terminal");
        let Some(PaneHold::CoreWaiting(waiting)) = terminal.hold.as_mut() else {
            panic!("the pane waits");
        };
        waiting.link.machine_cwd = Some(cwd.to_path_buf());
    }

    /// A folder of the machine `loop`, which this Mac does not have.
    fn far_folder() -> std::path::PathBuf {
        std::path::PathBuf::from("/far/on/loop/project")
    }

    fn why(app: &App, pane_id: PaneId) -> Option<CoreWaitReason> {
        app.state
            .pane_terminal(pane_id)
            .and_then(|terminal| terminal.core_waiting())
            .map(|waiting| waiting.why.clone())
    }

    fn has_runtime(app: &App, pane_id: PaneId) -> bool {
        app.find_pane(pane_id).is_some_and(|(_, pane)| {
            app.terminal_runtimes
                .get(&pane.attached_terminal_id)
                .is_some()
        })
    }

    fn respawn(app: &mut App, pane_id: PaneId, agent: bool) -> serde_json::Value {
        let params = PaneRespawnParams {
            pane_id: app.public_pane_id(0, pane_id).expect("public id"),
            force: true,
        };
        let response = app.handle_api_request(Request {
            id: "respawn".into(),
            method: if agent {
                Method::PaneRespawnAgent(params)
            } else {
                Method::PaneRespawn(params)
            },
        });
        serde_json::from_str(&response).expect("json")
    }

    fn split_on(app: &mut App, machine: &str) -> serde_json::Value {
        let target = app.state.workspaces[0].tabs[0].root_pane;
        split_with(app, target, Some(machine), None)
    }

    /// Splits `target` with a new pane, on `on` when given, in `cwd` when
    /// given (on the machine, for a pane there).
    fn split_with(
        app: &mut App,
        target: PaneId,
        on: Option<&str>,
        cwd: Option<&std::path::Path>,
    ) -> serde_json::Value {
        let target = app.public_pane_id(0, target);
        let response = app.handle_api_request(Request {
            id: "split".into(),
            method: Method::PaneSplit(crate::api::schema::PaneSplitParams {
                workspace_id: None,
                target_pane_id: target,
                direction: crate::api::schema::SplitDirection::Right,
                ratio: None,
                cwd: cwd.map(|cwd| cwd.display().to_string()),
                focus: false,
                right_click: Default::default(),
                env: Default::default(),
                on: on.map(str::to_string),
            }),
        });
        serde_json::from_str(&response).expect("json")
    }

    fn new_pane(app: &App, split: &serde_json::Value) -> PaneId {
        app.parse_pane_id(split["result"]["pane"]["pane_id"].as_str().expect("id"))
            .map(|(_, pane)| pane)
            .expect("the new pane")
    }

    /// The record core gives for `pane_id`'s terminal at `address`: its
    /// owner labels (D5), in `state`, with `extra` fields.
    fn owned_record(
        app: &App,
        pane_id: PaneId,
        address: &str,
        extra: serde_json::Value,
    ) -> serde_json::Value {
        let mut record = serde_json::json!({
            "address": address,
            "labels": {
                "herdr.session": format!("{}@fake", crate::pane::herdr_session_label()),
                "herdr.pane_id": app.public_pane_id(0, pane_id).expect("public id"),
                "herdr.launch": LAUNCH,
            },
        });
        for (key, value) in extra.as_object().expect("fields") {
            record[key] = value.clone();
        }
        record
    }

    fn kills(core: &FakeCore) -> usize {
        core.methods()
            .iter()
            .filter(|method| *method == "terminal.kill")
            .count()
    }

    fn connected(machine: &str) -> Probe {
        Probe {
            machine: name(machine),
            state: Some("connected".into()),
            leased: true,
            read: true,
        }
    }

    // C1 (D5): a respawn starts on this Mac, so a pane that waits for its
    // machine is refused, by every respawn path, and keeps its link.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pane_waiting_for_its_machine_is_never_respawned_here() {
        let (mut app, pane) = app_with_one_pane();
        wait_for_machine(&mut app, pane, "loop/t3", false);
        for agent in [false, true] {
            let answer = respawn(&mut app, pane, agent);
            assert_eq!(answer["error"]["code"], "pane_on_machine", "{answer}");
        }
        assert!(!app.respawn_pane_runtime(pane, crate::app::api::RespawnTarget::Shell));
        assert!(!has_runtime(&app, pane), "nothing started here");
        assert!(matches!(
            why(&app, pane),
            Some(CoreWaitReason::MachineAway { .. })
        ));
    }

    // C1, C3 (D5): a pane split onto a machine at runtime is watched by the
    // machine poll, and a respawn of it is refused: it runs on there.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_live_machine_pane_is_polled_and_never_respawned_here() {
        let core = FakeCore::start();
        std::env::set_var("HC_SOCKET", &core.socket);
        let (mut app, _) = app_with_one_pane();
        assert_eq!(app.core_machine_poll_deadline(), None);
        let split = split_on(&mut app, "loop");
        assert_eq!(split["result"]["pane"]["machine"], "loop", "{split}");
        assert!(
            app.core_machine_poll_deadline().is_some(),
            "the poll is armed"
        );
        let pane = app
            .parse_pane_id(split["result"]["pane"]["pane_id"].as_str().expect("id"))
            .map(|(_, pane)| pane)
            .expect("the new pane");
        let answer = respawn(&mut app, pane, false);
        assert_eq!(answer["error"]["code"], "pane_on_machine", "{answer}");
        let runs = core
            .methods()
            .iter()
            .filter(|method| *method == "terminal.run")
            .count();
        assert_eq!(runs, 1, "{:?}", core.methods());
        assert_eq!(app.pane_machine(pane).as_deref(), Some("loop"));
        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }

    // C2, N2 (D5): a waiting pane whose program core reports finished
    // ends with its link held, so the next save keeps the link and the next
    // restore never starts the pane here. Nothing is killed.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_ended_machine_pane_keeps_its_link_in_the_save() {
        let core = FakeCore::start();
        core.set_machine("loop", "connected");
        std::env::set_var("HC_SOCKET", &core.socket);
        let (mut app, pane) = app_with_one_pane();
        core.add_terminal(owned_record(
            &app,
            pane,
            "loop/t3",
            serde_json::json!({"state": "finished", "reason": "exited", "exit_code": 0}),
        ));
        wait_for_machine(&mut app, pane, "loop/t3", false);
        app.handle_core_machines(&[connected("loop")], Instant::now());
        assert!(
            matches!(why(&app, pane), Some(CoreWaitReason::MachineEnded { .. })),
            "{:?}",
            why(&app, pane)
        );
        assert!(!has_runtime(&app, pane));
        let saved = crate::persist::capture(
            &app.state.workspaces,
            &app.state.terminals,
            &app.terminal_runtimes,
            app.state.active,
            app.state.selected,
            None,
        );
        let saved_pane = &saved.workspaces[0].tabs[0].panes[&pane.raw()];
        assert_eq!(saved_pane.core_terminal, None, "never a local link");
        let link = saved_pane
            .core_machine_terminal
            .as_ref()
            .map(|link| link.address.clone());
        assert_eq!(link.as_deref(), Some("loop/t3"));
        assert_eq!(
            app.machine_waiting_panes().len(),
            0,
            "an ended pane is no longer polled for"
        );
        assert!(
            !core.methods().iter().any(|method| method == "terminal.run"),
            "{:?}",
            core.methods()
        );
        assert_eq!(kills(&core), 0, "{:?}", core.methods());
    }

    // N2 (D5, P5): core has the machine connected but lists no terminal of
    // the pane there, or one labelled for another pane: never final. The
    // pane waits with its terminal missing, is asked about again on the
    // next poll, and attaches once core lists its live terminal. Nothing is
    // started or killed, here or there.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_machine_pane_core_does_not_list_waits_and_is_asked_again() {
        let core = FakeCore::start();
        core.forget_terminal("loop/t3");
        core.set_machine("loop", "connected");
        std::env::set_var("HC_SOCKET", &core.socket);
        let (mut app, pane) = app_with_one_pane();
        wait_for_machine(&mut app, pane, "loop/t3", false);
        let missing = Some(CoreWaitReason::MachineMissing {
            machine: "loop".into(),
        });
        app.handle_core_machines(&[connected("loop")], Instant::now());
        assert_eq!(why(&app, pane), missing, "not listed");
        assert_eq!(app.machine_waiting_panes().len(), 1, "still polled for");
        // Listed, but labelled for another pane (the far herdr's own).
        core.add_terminal(serde_json::json!({"address": "loop/t3", "state": "live",
            "labels": {"herdr.session": crate::pane::herdr_session_label(), "herdr.pane_id": "w9-p9"}}));
        app.handle_core_machines(&[connected("loop")], Instant::now());
        assert_eq!(why(&app, pane), missing, "labelled for another pane");
        assert!(!has_runtime(&app, pane));
        core.add_terminal(owned_record(
            &app,
            pane,
            "loop/t3",
            serde_json::json!({"state": "live"}),
        ));
        app.handle_core_machines(&[connected("loop")], Instant::now());
        assert!(has_runtime(&app, pane), "attached once core lists it");
        assert_eq!(why(&app, pane), None);
        assert_eq!(kills(&core), 0, "{:?}", core.methods());
        assert!(
            !core.methods().iter().any(|method| method == "terminal.run"),
            "{:?}",
            core.methods()
        );
        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }

    // D5: a stop of this server that is not a restart ends this Mac's core
    // programs, never a program on a machine: only a person's close does.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_server_stop_leaves_a_machine_pane_running_there() {
        let core = FakeCore::start();
        std::env::set_var("HC_SOCKET", &core.socket);
        let (mut app, _) = app_with_one_pane();
        let split = split_on(&mut app, "loop");
        assert_eq!(split["result"]["pane"]["machine"], "loop", "{split}");
        app.keep_core_programs_for_stop(false);
        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
        assert_eq!(kills(&core), 0, "{:?}", core.methods());
    }

    /// What the build before machine panes (origin/master 7c273562, beta
    /// 212) does with one saved pane, transcribed from its code:
    /// `restore_tab` (src/persist/restore.rs:737) holds a pane whose `cwd`
    /// is no folder and that has a `core_terminal` as `DirectoryUnavailable`;
    /// its waiting retry gets `NotOurs` for an address with a machine
    /// (pane_io.rs:436) and then searches this Mac's terminals by the pane's
    /// labels (core_terminals.rs:193), attaching one it finds. A pane whose
    /// `cwd` is no folder and has no `core_terminal` only shows "Saved
    /// directory is unavailable": no hold, no probe, no search. A pane whose
    /// `cwd` exists starts fresh on this Mac when nothing answers for it.
    fn previous_beta_restores(pane: &serde_json::Value) -> &'static str {
        #[derive(serde::Deserialize)]
        struct OldLink {
            #[allow(dead_code)] // read as the old build reads it
            session: String,
            #[allow(dead_code)] // read as the old build reads it
            address: String,
        }
        // The old build's `PaneSnapshot` fields that decide a restore;
        // unknown fields, `core_machine_terminal` among them, are ignored.
        #[derive(serde::Deserialize)]
        struct OldPane {
            cwd: std::path::PathBuf,
            #[serde(default)]
            core_terminal: Option<OldLink>,
        }
        let old: OldPane = serde_json::from_value(pane.clone()).expect("the old build reads it");
        match (old.cwd.is_dir(), old.core_terminal.is_some()) {
            (false, true) => "held, then searched for by its labels here",
            (false, false) => "unavailable: no hold, no search, never started",
            (true, _) => "started fresh on this Mac",
        }
    }

    fn capture(app: &App) -> crate::persist::SessionSnapshot {
        crate::persist::capture(
            &app.state.workspaces,
            &app.state.terminals,
            &app.terminal_runtimes,
            app.state.active,
            app.state.selected,
            None,
        )
    }

    // N3 (D5): a saved pane on a machine is safe for the previous beta: its
    // link is saved as `core_machine_terminal`, which that build does not
    // know, and its `cwd` is a folder no host has, so that build shows the
    // pane unavailable without holding it, never searches for or attaches a
    // terminal of this Mac labelled with its id, and never starts it here.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_saved_machine_pane_is_never_started_by_the_previous_beta() {
        let core = FakeCore::start();
        core.set_machine("loop", "unreachable");
        std::env::set_var("HC_SOCKET", &core.socket);
        let (mut app, pane) = app_with_one_pane();
        wait_for_machine(&mut app, pane, "loop/t3", false);
        set_machine_cwd(&mut app, pane, &far_folder());
        let real = std::env::temp_dir();
        let json = serde_json::to_value(capture(&app)).expect("json");
        let pane_json = &json["workspaces"][0]["tabs"][0]["panes"][pane.raw().to_string()];
        assert_eq!(
            previous_beta_restores(pane_json),
            "unavailable: no hold, no search, never started",
            "{pane_json}"
        );
        // What this branch saved before: the link as `core_terminal`, which
        // the previous beta held and then attached to a terminal of this
        // Mac labelled with the pane's id; and before that the real folder,
        // which it started the pane in.
        let mut before = pane_json.clone();
        before["core_terminal"] = before["core_machine_terminal"].clone();
        assert_eq!(
            previous_beta_restores(&before),
            "held, then searched for by its labels here"
        );
        before["cwd"] = serde_json::json!(real);
        assert_eq!(previous_beta_restores(&before), "started fresh on this Mac");
        // This build keeps the folder there with the link, and never takes
        // it for one of this Mac: not the pane's, not the workspace's
        // (security review M1).
        let back: crate::persist::SessionSnapshot =
            serde_json::from_value(json).expect("this build reads it");
        let saved = &back.workspaces[0].tabs[0].panes[&pane.raw()];
        assert_eq!(
            saved.core_link().and_then(|link| link.link().machine_cwd),
            Some(far_folder())
        );
        assert_eq!(saved.local_cwd(), None);
        assert_ne!(back.workspaces[0].identity_cwd, far_folder());
    }

    // N3 (D5): the save of a machine pane carries its link only as
    // `core_machine_terminal`, with its folder there and no pid, while a
    // pane on this Mac saves exactly what it saved before.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_machine_link_is_saved_apart_from_a_local_one() {
        let core = FakeCore::start();
        std::env::set_var("HC_SOCKET", &core.socket);
        let (mut app, pane) = app_with_one_pane();
        let real = std::env::temp_dir();
        let local_link = CoreTerminalLink {
            program: Some(crate::platform::ProgramIdentity {
                pid: 4242,
                started: 7,
            }),
            launch: None,
            ..link_on("t3")
        };
        app.state.pane_terminal_mut(pane).expect("terminal").hold =
            Some(PaneHold::CoreWaiting(CoreWaiting {
                link: local_link,
                public_pane_id: app.public_pane_id(0, pane),
                why: CoreWaitReason::Unreachable,
            }));
        app.state.pane_terminal_mut(pane).expect("terminal").cwd = real.clone();
        let json = serde_json::to_value(capture(&app)).expect("json");
        let local = &json["workspaces"][0]["tabs"][0]["panes"][pane.raw().to_string()];
        assert_eq!(local["cwd"], serde_json::json!(real), "{local}");
        assert_eq!(
            local["core_terminal"],
            serde_json::json!({"session": crate::pane::core_session_name(),
                "address": "t3", "pid": 4242, "pid_started": 7}),
            "{local}"
        );
        assert!(local.get("core_machine_terminal").is_none(), "{local}");

        let machine_link = CoreTerminalLink {
            program: Some(crate::platform::ProgramIdentity {
                pid: 4242,
                started: 7,
            }),
            launch: Some("L1".into()),
            machine_cwd: Some(far_folder()),
            ..link_on("loop/t3")
        };
        app.state.pane_terminal_mut(pane).expect("terminal").hold =
            Some(PaneHold::CoreWaiting(CoreWaiting {
                link: machine_link,
                public_pane_id: app.public_pane_id(0, pane),
                why: CoreWaitReason::MachineAway {
                    machine: "loop".into(),
                    dormant: false,
                },
            }));
        let json = serde_json::to_value(capture(&app)).expect("json");
        let far = &json["workspaces"][0]["tabs"][0]["panes"][pane.raw().to_string()];
        assert_eq!(
            far["cwd"], "/dev/null/herdr-pane-on-a-core-machine",
            "{far}"
        );
        assert!(far.get("core_terminal").is_none(), "{far}");
        assert_eq!(
            far["core_machine_terminal"],
            serde_json::json!({"session": crate::pane::core_session_name(),
                "address": "loop/t3", "machine_cwd": far_folder(), "launch": "L1"}),
            "{far}"
        );
    }

    // N3 (D5): a machine pane this build saved restores on this build as a
    // pane waiting for its machine, with its folder there: nothing starts,
    // and a terminal of this Mac labelled with its id is not searched for.
    // Saved again, it keeps its link apart from `core_terminal`.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_saved_machine_pane_restores_waiting_for_its_machine() {
        let core = FakeCore::start();
        core.forget_terminal("loop/t3");
        core.set_machine("loop", "unreachable");
        std::env::set_var("HC_SOCKET", &core.socket);
        let (mut app, pane) = app_with_one_pane();
        let public_pane_id = app.public_pane_id(0, pane).expect("public id");
        core.add_terminal(serde_json::json!({"address": "t9", "state": "live",
            "labels": {"herdr.session": crate::pane::herdr_session_label(),
                "herdr.pane_id": public_pane_id}}));
        wait_for_machine(&mut app, pane, "loop/t3", false);
        set_machine_cwd(&mut app, pane, &far_folder());
        let json = serde_json::to_string(&capture(&app)).expect("json");
        let mut snapshot: crate::persist::SessionSnapshot =
            serde_json::from_str(&json).expect("this build reads it");
        for _ in 0..2 {
            let (events, _rx) = tokio::sync::mpsc::channel(32);
            let (workspaces, terminals, runtimes) = crate::persist::restore(
                &snapshot,
                None,
                24,
                80,
                0,
                "/bin/sh",
                crate::config::ShellModeConfig::NonLogin,
                false,
                events,
                std::sync::Arc::new(tokio::sync::Notify::new()),
                std::sync::Arc::new(crate::render_signal::RenderSignal::new()),
            );
            let runtimes = crate::terminal::TerminalRuntimeRegistry::from(runtimes);
            let root = workspaces[0].tabs[0].root_pane;
            let terminal = &terminals[workspaces[0].tabs[0].terminal_id(root).expect("id")];
            assert!(runtimes.get(&terminal.id).is_none(), "nothing started");
            let waiting = terminal.core_waiting().expect("waits");
            assert_eq!(waiting.link.address, "loop/t3");
            assert!(
                matches!(waiting.why, CoreWaitReason::MachineAway { .. }),
                "{:?}",
                waiting.why
            );
            assert_eq!(
                waiting.link.machine_cwd,
                Some(far_folder()),
                "its folder there, with its link"
            );
            assert_ne!(terminal.cwd, far_folder(), "never a folder of this Mac");
            snapshot =
                crate::persist::capture(&workspaces, &terminals, &runtimes, Some(0), 0, None);
            let saved = &snapshot.workspaces[0].tabs[0].panes[&root.raw()];
            assert_eq!(saved.core_terminal, None);
            assert_eq!(
                saved
                    .core_machine_terminal
                    .as_ref()
                    .map(|link| link.address.as_str()),
                Some("loop/t3")
            );
        }
        let methods = core.methods();
        assert!(
            !methods
                .iter()
                .any(|method| method == "terminal.list" || method == "terminal.run"),
            "no search, nothing started: {methods:?}"
        );
    }

    // Security review M1: `--cwd` on a machine names a folder there. It
    // goes to the machine and stays with the pane's link, and it is never
    // a folder of this Mac: not the pane's here, nor the folder a local
    // split of the pane starts in, even when this Mac has a folder of that
    // name.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_machine_folder_never_becomes_a_folder_here() {
        let core = FakeCore::start();
        std::env::set_var("HC_SOCKET", &core.socket);
        let (mut app, root) = app_with_one_pane();
        let here_too = std::env::temp_dir().join(format!("h308-far-{}", std::process::id()));
        std::fs::create_dir_all(&here_too).expect("a folder of that name here");
        let split = split_with(&mut app, root, Some("loop"), Some(&here_too));
        let pane = new_pane(&app, &split);
        let run = core
            .requests()
            .into_iter()
            .find(|request| request["method"] == "terminal.run")
            .expect("run");
        assert_eq!(
            run["params"]["cwd"],
            serde_json::json!(here_too),
            "sent there"
        );
        let terminal = app.state.pane_terminal(pane).expect("terminal");
        assert_ne!(terminal.cwd, here_too, "not its folder here");
        let link = app
            .terminal_runtimes
            .get(&terminal.id)
            .and_then(crate::terminal::TerminalRuntime::core_terminal)
            .expect("link");
        assert_eq!(
            link.machine_cwd,
            Some(here_too.clone()),
            "kept with its link"
        );

        let local = split_with(&mut app, pane, None, None);
        let local = new_pane(&app, &local);
        assert_ne!(
            app.state.pane_terminal(local).expect("terminal").cwd,
            here_too,
            "a local split does not start there"
        );
        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.set_keep_core_terminal(true);
            runtime.shutdown();
        }
        let _ = std::fs::remove_dir_all(here_too);
    }

    // Third read, finding 6: a worktree removal stops the runtimes of its
    // workspace's panes on this Mac; a program on a machine holds no folder
    // here, and only a person's close ends it, so it runs on.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_worktree_removal_leaves_a_machine_pane_running() {
        let core = FakeCore::start();
        std::env::set_var("HC_SOCKET", &core.socket);
        let (mut app, _) = app_with_one_pane();
        let split = split_on(&mut app, "loop");
        let pane = new_pane(&app, &split);
        let stopped = app.shutdown_workspace_terminal_runtimes_for_worktree_remove(0);
        assert!(!stopped.contains(&pane), "{stopped:?}");
        assert!(has_runtime(&app, pane));
        assert_eq!(kills(&core), 0, "{:?}", core.methods());
        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.set_keep_core_terminal(true);
            runtime.shutdown();
        }
    }

    // Third read, finding 6: whatever the pane is, a suspend cause on a
    // machine pane makes it wait with its link, never close and leave its
    // program there without one.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_machine_pane_waits_even_where_a_pane_would_close() {
        let core = FakeCore::start();
        std::env::set_var("HC_SOCKET", &core.socket);
        let (mut app, root) = app_with_one_pane();
        let split = split_on(&mut app, "loop");
        let pane = new_pane(&app, &split);
        app.overlay_panes.insert(
            pane,
            crate::app::OverlayPaneState {
                ws_idx: 0,
                tab_idx: 0,
                previous_focus: root,
                previous_zoomed: false,
                temp_files: Vec::new(),
            },
        );
        app.handle_internal_event(crate::events::AppEvent::PaneDied {
            pane_id: pane,
            exit_reason: crate::platform::ChildExitReason::Suspended(
                crate::platform::SuspendCause::CoreStopped,
            ),
        });
        assert!(app.find_pane(pane).is_some(), "the pane stays");
        assert!(matches!(
            why(&app, pane),
            Some(CoreWaitReason::MachineAway { .. })
        ));
        assert_eq!(kills(&core), 0, "{:?}", core.methods());
    }

    // N4 (D5): the idle move refuses a pane on a machine by name: its kill
    // would end the program there and start the replacement here.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_idle_move_refuses_a_machine_pane() {
        let core = FakeCore::start();
        std::env::set_var("HC_SOCKET", &core.socket);
        let (mut app, _) = app_with_one_pane();
        let split = split_on(&mut app, "loop");
        let pane = app
            .parse_pane_id(split["result"]["pane"]["pane_id"].as_str().expect("id"))
            .map(|(_, pane)| pane)
            .expect("the new pane");
        for check_only in [true, false] {
            assert_eq!(
                app.respawn_pane_if_idle(pane, check_only),
                [crate::api::schema::PaneMoveRefusal::OnMachine]
            );
        }
        assert!(has_runtime(&app, pane), "left running");
        assert_eq!(kills(&core), 0, "{:?}", core.methods());
        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }

    // N4 (D5): a suspended pane on a machine is never resumed here, by its
    // agent's resume plan or its launch command: it waits for its machine
    // with the link its suspension kept.
    #[test]
    fn a_suspended_machine_pane_is_never_resumed_here() {
        for agent in [true, false] {
            let (mut app, pane) = app_with_one_pane();
            let terminal = app.state.pane_terminal_mut(pane).expect("terminal");
            if agent {
                terminal.set_agent_name("codex".into());
                terminal.set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
                    source: "herdr:codex".into(),
                    agent: "codex".into(),
                    session_ref: crate::agent_resume::AgentSessionRef::id("codex-session")
                        .expect("id"),
                });
            }
            let mut suspension =
                crate::terminal::state::Suspension::new(crate::platform::SuspendCause::HolderLost);
            suspension.core_link = Some(link_on("loop/t3"));
            app.state.suspend_pane(pane, Instant::now(), suspension);
            let _ = app.render_dirty.take();
            assert!(!app.queue_resume(pane, Default::default()), "agent {agent}");
            assert!(
                app.render_dirty.take().generic,
                "the machine wait is drawn (third read, finding 7)"
            );
            assert!(!app.resume_queue.busy(), "nothing queued");
            assert!(!has_runtime(&app, pane));
            let terminal = app.state.pane_terminal(pane).expect("terminal");
            assert_eq!(
                terminal.held_core_link().map(|link| link.address),
                Some("loop/t3".into()),
                "the link is kept"
            );
            assert!(matches!(
                why(&app, pane),
                Some(CoreWaitReason::MachineAway { .. })
            ));
        }
    }

    // C5 (D5, P5): a poll said connected, but core has not linked the
    // machine yet when the pane asks: its `not_found` is not final, and the
    // pane keeps waiting.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_waiting_pane_whose_terminal_core_left_out_keeps_waiting() {
        let core = FakeCore::start();
        core.forget_terminal("loop/t3");
        core.set_machine("loop", "connecting");
        std::env::set_var("HC_SOCKET", &core.socket);
        let (mut app, pane) = app_with_one_pane();
        wait_for_machine(&mut app, pane, "loop/t3", false);
        app.handle_core_machines(&[connected("loop")], Instant::now());
        assert!(matches!(
            why(&app, pane),
            Some(CoreWaitReason::MachineAway { .. })
        ));
    }

    // C4 (D5): a `machine.list` that failed reads nothing: the machine keeps
    // its mark and its panes their reason.
    #[test]
    fn an_unread_probe_changes_neither_the_machine_nor_its_panes() {
        let (mut app, pane) = app_with_one_pane();
        wait_for_machine(&mut app, pane, "unread/t3", true);
        let link = panes::link(&name("unread"));
        link.set_connected(true);
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

    // C6 (D5): a stop of this Mac's core ends a machine pane's stream with
    // a suspend cause; the pane waits for its machine with its link, and is
    // neither suspended (which starts it again here) nor closed.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_suspend_cause_makes_a_machine_pane_wait_for_its_machine() {
        let core = FakeCore::start();
        std::env::set_var("HC_SOCKET", &core.socket);
        let (mut app, _) = app_with_one_pane();
        let split = split_on(&mut app, "loop");
        let pane = app
            .parse_pane_id(split["result"]["pane"]["pane_id"].as_str().expect("id"))
            .map(|(_, pane)| pane)
            .expect("the new pane");
        app.handle_internal_event(crate::events::AppEvent::PaneDied {
            pane_id: pane,
            exit_reason: crate::platform::ChildExitReason::Suspended(
                crate::platform::SuspendCause::CoreStopped,
            ),
        });
        assert!(app.find_pane(pane).is_some(), "the pane stays");
        let terminal = app.state.pane_terminal(pane).expect("terminal");
        assert!(!terminal.is_suspended());
        assert_eq!(
            terminal
                .core_waiting()
                .map(|waiting| waiting.link.address.as_str()),
            Some("loop/t1")
        );
        assert!(matches!(
            why(&app, pane),
            Some(CoreWaitReason::MachineAway { .. })
        ));
        assert!(!has_runtime(&app, pane));
        assert!(!panes::link(&name("loop")).connected());
        assert!(app.state.suspended_panes().is_empty());
        // N1: the stream ended, the program it waits for may run on there.
        assert!(
            !core
                .methods()
                .iter()
                .any(|method| method == "terminal.kill"),
            "the far program is never killed: {:?}",
            core.methods()
        );
        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }
}
