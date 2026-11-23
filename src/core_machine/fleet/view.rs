//! A view's structural actions (core-phase5 D7, E.3): new tab, split, close
//! pane and rename in `herdr server --view <m>` go to the far herdr as one
//! API request each, through the fleet CLI's reviewed path: `machine.exec`
//! of D5's `env -u` wrapper and `<far herdr> --session S remote-api-bridge
//! --request`, with the request line as its input. The argv is fixed; every
//! name travels inside the JSON request as one string, so none reaches a
//! shell or an argument parser there (V20, D11). The far herdr changes its
//! layout and republishes it; the view follows its document.
//!
//! A person's action in a view is a person's act on that machine (D3), but
//! it is no dial: on a machine that is not connected it is refused at once,
//! with nothing written, sent or dialled (D7's rule for a call without
//! `--dial`); a key in one of the view's panes is what wakes it (D6). On a
//! connected machine the view's own `cli-<pid>` keep lease holds the link
//! for the call and admits no dial, so a drop during it is not redialled on
//! its account. A refusal or a far error is the action's own answer: the
//! client shows it as a notice and the view stays connected.

use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

use super::{
    bridge_argv, check_request_line, far_herdr, far_herdr_session, lock, CoreAt, FarRoute,
};
use crate::core_machine::state::{now_unix, passwd_machines_root, LeaseHolder, MachineDir};
use crate::core_machine::MachineName;

/// How long the far herdr gets for one action, so the far herdr probe
/// (when its answer is stale) and the action both end within the client's
/// own 60 s wait for an endpoint command.
pub(crate) const VIEW_ACTION_SECS: u64 = 20;

/// The view's keep lease lives this long past its one write: longer than
/// any action (the probe's 30 s and the action's 20 s, with core's
/// margins), within a CLI lease's 60 s.
const VIEW_LEASE_SECS: f64 = super::LEASE_LIFE_SECS;

/// One action at a time in a view process: they share its one lease file.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// Sends one API request line (`{"id", "method", "params"}` and a newline)
/// to the far herdr of `machine` and returns its one reply line, or why
/// there is none: refused unsent, or failed after sending (then the message
/// says that it may have run there).
pub(crate) fn send(machine: &MachineName, line: &[u8]) -> Result<Vec<u8>, String> {
    send_at(
        &CoreAt::primary(),
        passwd_machines_root(),
        machine,
        line,
        &LeaseHolder::cli(std::process::id()),
    )
}

pub(super) fn send_at(
    core: &CoreAt,
    root: io::Result<PathBuf>,
    name: &MachineName,
    line: &[u8],
    holder: &LeaseHolder,
) -> Result<Vec<u8>, String> {
    let _one = lock(&ONE_AT_A_TIME);
    let not_sent = |why: String| format!("{why}; nothing was sent to {name}");
    let root = root.map_err(|err| not_sent(format!("cannot find herdr's machine state: {err}")))?;
    let dir = MachineDir::at(&root, name);
    if !dir.target_path().exists() {
        return Err(not_sent(format!("herdr here did not add {name}")));
    }
    dir.check()
        .map_err(|err| not_sent(format!("{name}: {err}")))?;
    let target = dir
        .read_target()
        .map_err(|err| not_sent(format!("{name}: {err}")))?;
    let session = far_herdr_session(&target.session).ok_or_else(|| {
        not_sent(format!(
            "{name} links to Herdr Core session {}, which is no herdr session's own",
            target.session
        ))
    })?;
    let record = match core.record(name) {
        Ok(Some(record)) => record,
        Ok(None) => {
            return Err(not_sent(format!(
                "herdr-core here does not list {name}, or is not running"
            )))
        }
        Err(err) => {
            return Err(not_sent(format!(
                "herdr-core did not list its machines: {err}"
            )))
        }
    };
    // The same route check as the fleet CLI's (security review M5): core's
    // machine of that name must be the one herdr added.
    if !crate::core_machine::command::runs_herdr_link(&record, name, &target) {
        return Err(not_sent(format!(
            "herdr-core's machine {name} is not the one herdr added"
        )));
    }
    let state = record["state"].as_str().unwrap_or("unknown");
    if state != "connected" {
        return Err(not_connected(name, state));
    }
    check_request_line(name, line)?;
    let _lease = KeepLease::write(&dir, holder)
        .map_err(|err| not_sent(format!("{name}: could not write the lease: {err}")))?;
    let far = far_herdr(core, name, &dir, &session, &target.session)?;
    let argv = bridge_argv(&far, &session);
    FarRoute {
        core,
        name,
        dir: &dir,
        argv: &argv,
        far_path: &far.path,
    }
    .send(line, VIEW_ACTION_SECS)
}

/// Why an action on a machine that is not connected is refused: nothing
/// was written, sent or dialled (D7, F7).
fn not_connected(name: &MachineName, state: &str) -> String {
    let what = match state {
        "needs_login" => format!("{name} needs a login (herdr machine login {name})"),
        "connecting" => format!("{name} is still connecting"),
        "dormant" | "unreachable" | "unknown" => {
            format!("{name} is dormant: press a key in one of its panes to wake it")
        }
        other => format!("{name} is not connected ({other})"),
    };
    format!("{what}, then do this again; nothing was sent and nothing dialled")
}

/// The view's `cli-<pid>` keep lease for one action: written once, removed
/// when the action ends. It is not renewed: an action ends long before it
/// does, and a view killed during one leaves a lease that keeps an up link
/// a minute at most and admits no dial. No termination handler is put in
/// the view server for it (the fleet CLI's would end the server on SIGHUP).
struct KeepLease<'a> {
    dir: &'a MachineDir,
    holder: &'a LeaseHolder,
}

impl<'a> KeepLease<'a> {
    fn write(dir: &'a MachineDir, holder: &'a LeaseHolder) -> io::Result<Self> {
        dir.write_keep_lease(holder, now_unix() + VIEW_LEASE_SECS)?;
        Ok(Self { dir, holder })
    }
}

impl Drop for KeepLease<'_> {
    fn drop(&mut self) {
        if let Err(err) = self.dir.remove_lease(self.holder) {
            tracing::warn!(%err, "could not remove the view action's lease");
        }
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use serde_json::{json, Value};

    use super::*;
    use crate::core_machine::fleet::FarHerdr;
    use crate::core_machine::state::{tests::scratch, write_atomic, DialRecord, Target};
    use crate::pane::core_machines as core;
    use crate::pane::FakeCore;

    fn machine() -> MachineName {
        MachineName::parse("loop").expect("name")
    }

    fn far() -> FarHerdr {
        FarHerdr {
            link_at: 100.5,
            link_pid: 31,
            probed_at: now_unix(),
            session: "default".into(),
            path: "/opt/homebrew/bin/herdr".into(),
            version: Some("0.9.3".into()),
            protocol: Some(30),
            endpoint_capabilities: Vec::new(),
            core_session: Some("default".into()),
            request_bridge: true,
        }
    }

    /// A machine herdr added, linked now, whose far herdr was probed on
    /// this link (so the action is the only exec).
    fn added(tag: &str) -> (PathBuf, MachineDir) {
        let root = scratch(tag);
        let dir = MachineDir::at(&root, &machine());
        dir.create().expect("create");
        dir.write_target(&Target {
            target: "ac@loop".into(),
            session: "default".into(),
            ssh: "/usr/bin/ssh".into(),
            ssh_config: None,
            ssh_auth_sock: None,
            via: None,
        })
        .expect("target");
        dir.write_dial_record(&DialRecord { at: 100.5, pid: 31 })
            .expect("dial");
        write_atomic(
            &dir.far_herdr_path(),
            &serde_json::to_vec(&far()).expect("json"),
        )
        .expect("probe");
        (root, dir)
    }

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn execs(core: &FakeCore) -> Vec<Value> {
        core.requests()
            .into_iter()
            .filter(|request| request["method"] == "machine.exec")
            .collect()
    }

    fn holder() -> LeaseHolder {
        LeaseHolder::cli(std::process::id())
    }

    // V20 (D7, D11): a workspace named `$(touch x); w`, renamed from a
    // view. The argv is the fixed wrapper and far bridge, with no shell in
    // it and no element built from the name; the name is one JSON string of
    // the request the bridge reads as its input, so nothing there parses it
    // as script or as an option.
    #[test]
    fn v20_a_name_reaches_the_far_herdr_as_data_never_a_shell_line() {
        let name = "$(touch x); w";
        let request = json!({"id": "view:1", "method": "workspace.rename",
                             "params": {"workspace_id": "w3", "label": name}});
        let line = format!("{request}\n").into_bytes();
        let far_reply = b"{\"id\":\"view:1\",\"result\":{\"type\":\"ok\"}}\n";
        let core = FakeCore::start_with(|config| {
            config.exec_reply = Some(json!({"stdout": b64(far_reply), "stderr": "",
                "truncated": false, "timed_out": false, "ms": 3, "code": 0}))
        });
        core.set_machine("loop", "connected");
        let (root, dir) = added("view-v20");

        let reply = send_at(
            &CoreAt::at(&core.socket),
            Ok(root.clone()),
            &machine(),
            &line,
            &holder(),
        )
        .expect("sent");
        assert_eq!(reply, far_reply);

        let sent = execs(&core);
        assert_eq!(sent.len(), 1, "one exec, the action");
        let params = &sent[0]["params"];
        assert_eq!(params["on"], "loop");
        assert_eq!(params["timeout"], VIEW_ACTION_SECS);
        let argv: Vec<String> = serde_json::from_value(params["argv"].clone()).expect("argv");
        let mut expected = core::machine_env_wrapper();
        expected.extend(
            [
                "/opt/homebrew/bin/herdr",
                "--session",
                "default",
                "remote-api-bridge",
                "--request",
            ]
            .map(String::from),
        );
        assert_eq!(argv, expected);
        assert!(
            !argv.iter().any(|arg| arg == "sh"
                || arg.ends_with("/sh")
                || arg == "-c"
                || arg.contains("touch")),
            "no shell and no name in the argv: {argv:?}"
        );
        let input = base64::engine::general_purpose::STANDARD
            .decode(params["input"].as_str().expect("input"))
            .expect("base64");
        assert_eq!(input, line, "the request line, byte for byte");
        let sent_request: Value = serde_json::from_slice(&input).expect("json");
        assert_eq!(sent_request["params"]["label"], name, "one string");

        assert!(
            !core
                .methods()
                .iter()
                .any(|method| method == "machine.connect"),
            "an action never dials"
        );
        assert!(dir.leases().is_empty(), "the action's lease is gone");
        let _ = std::fs::remove_dir_all(&root);
    }

    // D7, D3: an action on a machine that is not connected is refused at
    // once: no lease, no exec, no dial; a key in a pane is what wakes it.
    #[test]
    fn an_action_on_a_dormant_machine_refuses_and_dials_nothing() {
        let core = FakeCore::start();
        core.set_machine("loop", "dormant");
        let (root, dir) = added("view-dormant");
        let line = b"{\"id\":\"view:2\",\"method\":\"tab.create\",\"params\":{\"focus\":false}}\n";
        let err = send_at(
            &CoreAt::at(&core.socket),
            Ok(root.clone()),
            &machine(),
            line,
            &holder(),
        )
        .expect_err("refused");
        assert!(err.contains("loop is dormant: press a key"), "{err}");
        assert!(
            err.contains("nothing was sent and nothing dialled"),
            "{err}"
        );
        assert!(execs(&core).is_empty());
        assert!(!core
            .methods()
            .iter()
            .any(|method| method == "machine.connect"));
        assert!(dir.leases().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    // A far error is a reply like any other (the far herdr ran and said
    // no): passed on whole, for the view to make it the action's notice.
    #[test]
    fn a_far_refusal_comes_back_as_the_reply() {
        let far_reply =
            b"{\"id\":\"view:3\",\"error\":{\"code\":\"unknown_method\",\"message\":\"unknown method: tab.create\"}}\n";
        let core = FakeCore::start_with(|config| {
            config.exec_reply = Some(json!({"stdout": b64(far_reply), "stderr": "",
                "truncated": false, "timed_out": false, "ms": 3, "code": 0}))
        });
        core.set_machine("loop", "connected");
        let (root, _dir) = added("view-far-error");
        let line = b"{\"id\":\"view:3\",\"method\":\"tab.create\",\"params\":{\"focus\":false}}\n";
        let reply = send_at(
            &CoreAt::at(&core.socket),
            Ok(root.clone()),
            &machine(),
            line,
            &holder(),
        )
        .expect("a reply");
        assert_eq!(reply, far_reply);
        let _ = std::fs::remove_dir_all(&root);
    }

    // The view's lease for an action keeps the link and admits no dial,
    // and goes when the action ends.
    #[test]
    fn the_actions_lease_keeps_the_link_and_admits_no_dial() {
        let (root, dir) = added("view-lease");
        let holder = holder();
        {
            let _lease = KeepLease::write(&dir, &holder).expect("lease");
            let leases = dir.leases();
            assert_eq!(leases.len(), 1);
            assert_eq!(leases[0].holder, holder);
            assert!(!leases[0].dials, "a view action's lease admits a dial");
        }
        assert!(dir.leases().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    // A name herdr did not add, or a missing state folder, sends nothing.
    #[test]
    fn a_machine_herdr_did_not_add_gets_nothing() {
        let core = FakeCore::start();
        core.set_machine("loop", "connected");
        let root = scratch("view-not-added");
        let line = b"{\"id\":\"view:4\",\"method\":\"tab.create\",\"params\":{\"focus\":false}}\n";
        let err = send_at(
            &CoreAt::at(&core.socket),
            Ok(root.clone()),
            &machine(),
            line,
            &holder(),
        )
        .expect_err("refused");
        assert!(err.contains("herdr here did not add loop"), "{err}");
        assert!(execs(&core).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
