//! Live checks of the herdr-core backend against a real `hc`
//! (openspec/changes/core-backend, design.md § Verifier). Ignored unless
//! `HERDR_TEST_HC=1`: they start a throwaway core of their own, with its state
//! under a short `/tmp` folder, and never touch a user's core. Nextest runs
//! each test in its own process, so the process-wide backend choice and
//! `HC_SOCKET` set here stay inside one test.

use super::runtime_characterization_tests::{
    next_pane_died, spawn_sh, wait_for_file, wait_until_gone, Scratch, WAIT,
};
use super::*;
use std::path::PathBuf;
use std::time::Duration;

/// A throwaway core: its own session and state folder, started with
/// variables herdr strips (`TMUX`, `CLAUDECODE`) so V2 can check they stay
/// stripped.
struct ScratchCore {
    state: PathBuf,
    session: String,
}

impl ScratchCore {
    fn start() -> Option<Self> {
        std::env::var_os("HERDR_TEST_HC")?;
        let state = PathBuf::from(format!("/tmp/hct-{}", std::process::id()));
        std::fs::create_dir_all(&state).expect("scratch core state");
        let session = format!("t{}", std::process::id());
        let status = std::process::Command::new("hc")
            .args(["server", "start"])
            .env("HC_SESSION", &session)
            .env("XDG_STATE_HOME", &state)
            .env_remove("XDG_RUNTIME_DIR")
            .env("TMUX", "/tmp/tmux-leak,1,0")
            .env("CLAUDECODE", "1")
            // An agent's credentials, as a core an agent started would keep
            // (security review M1).
            .env("ANTHROPIC_AUTH_TOKEN", "canary-auth")
            .env("CLAUDE_CODE_OAUTH_TOKEN", "canary-oauth")
            .status()
            .expect("run hc");
        assert!(status.success(), "hc server start failed");
        let socket = state
            .join("herdr-core")
            .join(&session)
            .join(format!("{session}.sock"));
        std::env::set_var("HC_SOCKET", &socket);
        crate::pane::set_preferred_runtime(true);
        Some(Self { state, session })
    }

    fn pids(&self) -> (u32, Vec<u32>) {
        let out = std::process::Command::new("hc")
            .args(["server", "status", "--json"])
            .env("HC_SESSION", &self.session)
            .env("XDG_STATE_HOME", &self.state)
            .output()
            .expect("hc server status");
        let status: serde_json::Value = serde_json::from_slice(&out.stdout).expect("status json");
        let holders = status["holders"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|holder| holder["pid"].as_u64().map(|pid| pid as u32))
            .collect();
        (status["pid"].as_u64().unwrap_or(0) as u32, holders)
    }
}

impl ScratchCore {
    /// The terminal's `status` as core keeps it (`Null` when it has none).
    fn status(&self, address: &str) -> serde_json::Value {
        let out = std::process::Command::new("hc")
            .args(["get", address, "--json"])
            .env("HC_SESSION", &self.session)
            .env("XDG_STATE_HOME", &self.state)
            .output()
            .expect("hc get");
        let record: serde_json::Value = serde_json::from_slice(&out.stdout).expect("get json");
        record["status"].clone()
    }

    async fn wait_for_status(
        &self,
        address: &str,
        done: impl Fn(&serde_json::Value) -> bool,
    ) -> serde_json::Value {
        tokio::time::timeout(WAIT, async {
            loop {
                let status = self.status(address);
                if done(&status) {
                    return status;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("core status: last {}", self.status(address)))
    }

    fn restart(&self) {
        let status = std::process::Command::new("hc")
            .args(["server", "restart"])
            .env("HC_SESSION", &self.session)
            .env("XDG_STATE_HOME", &self.state)
            .stdout(std::process::Stdio::null())
            .status()
            .expect("hc server restart");
        assert!(status.success(), "hc server restart failed");
    }
}

impl Drop for ScratchCore {
    fn drop(&mut self) {
        let _ = std::process::Command::new("hc")
            .args(["server", "stop"])
            .env("HC_SESSION", &self.session)
            .env("XDG_STATE_HOME", &self.state)
            .status();
        let _ = std::fs::remove_dir_all(&self.state);
    }
}

fn is_core(runtime: &PaneRuntime) -> bool {
    matches!(runtime.io, PaneIo::Core(_))
}

// V2: argv, cwd and env reach a core pane's program, and variables herdr
// strips stay stripped although the core's own environment has them.
#[tokio::test]
#[ignore = "needs hc; set HERDR_TEST_HC=1"]
async fn core_v2_spawn_contract_and_stripped_keys() {
    let Some(_core) = ScratchCore::start() else {
        return;
    };
    let scratch = Scratch::new("core-v2");
    let env = PaneLaunchEnv::from_extra(vec![
        ("HERDR_SEAM_PROBE".into(), "v2-probe".into()),
        ("ANTHROPIC_API_KEY".into(), "set-by-the-pane".into()),
    ]);
    let (runtime, _events) = spawn_sh(
        &scratch.0,
        r#"printf '%s|%s|%s|%s|%s|%s|%s|%s\n' "$0" "$(pwd -P)" "$HERDR_SEAM_PROBE" "${TMUX-unset}" "${CLAUDECODE-unset}" "${ANTHROPIC_AUTH_TOKEN-unset}" "${CLAUDE_CODE_OAUTH_TOKEN-unset}" "${ANTHROPIC_API_KEY-unset}" > out.txt"#,
        &["seam-arg0"],
        &env,
        24,
        80,
        0,
    );
    assert!(is_core(&runtime), "the pane runs on the core backend");
    let out = wait_for_file(&scratch.path("out.txt"), |text| text.ends_with('\n')).await;
    runtime.shutdown();
    assert_eq!(
        out,
        format!(
            "seam-arg0|{}|v2-probe|unset|unset|unset|unset|set-by-the-pane\n",
            scratch.0.display()
        )
    );
}

// V3 (order): 20 000 numbered lines reach the parser in order through core.
#[tokio::test]
#[ignore = "needs hc; set HERDR_TEST_HC=1"]
async fn core_v3_output_reaches_the_parser_in_order() {
    let Some(_core) = ScratchCore::start() else {
        return;
    };
    const LINES: usize = 20_000;
    let scratch = Scratch::new("core-v3");
    let (runtime, _events) = spawn_sh(
        &scratch.0,
        "i=1; while [ $i -le 20000 ]; do echo $i; i=$((i+1)); done; echo SEAM-DONE; exec sleep 30",
        &[],
        &PaneLaunchEnv::default(),
        24,
        80,
        16 * 1024 * 1024,
    );
    assert!(is_core(&runtime));
    let text = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let text = runtime.recent_text_snapshot(LINES + 100).text;
            if text.contains("SEAM-DONE") {
                break text;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the program finished printing");
    runtime.shutdown();
    let numbers: Vec<usize> = text
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect();
    assert_eq!(numbers, (1..=LINES).collect::<Vec<_>>());
}

// V4: plain writes, then a queued submission, arrive text then Enter.
#[tokio::test]
#[ignore = "needs hc; set HERDR_TEST_HC=1"]
async fn core_v4_input_and_queued_submission_arrive_in_order() {
    let Some(_core) = ScratchCore::start() else {
        return;
    };
    let scratch = Scratch::new("core-v4");
    let (runtime, _events) = spawn_sh(
        &scratch.0,
        "stty raw -echo\necho ready > ready\ndd bs=1 count=5 2>/dev/null > in.bin\nexec sleep 30",
        &[],
        &PaneLaunchEnv::default(),
        24,
        80,
        0,
    );
    assert!(is_core(&runtime));
    wait_for_file(&scratch.path("ready"), |text| text.ends_with('\n')).await;
    runtime
        .try_send_bytes(Bytes::from_static(b"ab"))
        .expect("plain write");
    let submitted = runtime
        .queue_user_input_submission(
            Bytes::from_static(b"cd"),
            Bytes::from_static(b"\r"),
            Duration::from_millis(50),
            None,
        )
        .expect("queue submission");
    let result = tokio::task::spawn_blocking(move || submitted.recv_timeout(WAIT))
        .await
        .unwrap();
    let input = tokio::time::timeout(WAIT, async {
        loop {
            if let Ok(bytes) = std::fs::read(scratch.path("in.bin")) {
                if bytes.len() == 5 {
                    break bytes;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the program read five bytes");
    runtime.shutdown();
    assert!(
        matches!(result, Ok(Ok(()))),
        "submission reported {result:?}"
    );
    assert_eq!(input, b"abcd\r");
}

// V5: a resize reaches the program, and a cursor query after it sees the size.
#[tokio::test]
#[ignore = "needs hc; set HERDR_TEST_HC=1"]
async fn core_v5_resize_and_queries_see_the_new_size() {
    let Some(_core) = ScratchCore::start() else {
        return;
    };
    let scratch = Scratch::new("core-v5");
    let (runtime, _events) = spawn_sh(
        &scratch.0,
        "echo ready > ready\nwhile [ ! -e go ]; do sleep 0.02; done\nstty size > size.txt\nstty raw -echo\nprintf '\\033[999;999H\\033[6n'\ndd bs=1 count=9 2>/dev/null | od -An -c > reply.txt\nexec sleep 30",
        &[],
        &PaneLaunchEnv::default(),
        24,
        80,
        0,
    );
    assert!(is_core(&runtime));
    wait_for_file(&scratch.path("ready"), |text| text.ends_with('\n')).await;
    runtime.resize(30, 100, 0, 0);
    tokio::time::sleep(Duration::from_millis(200)).await;
    std::fs::write(scratch.path("go"), "").unwrap();
    let size = wait_for_file(&scratch.path("size.txt"), |text| text.ends_with('\n')).await;
    let reply = wait_for_file(&scratch.path("reply.txt"), |text| text.contains('R')).await;
    runtime.shutdown();
    assert_eq!(size, "30 100\n");
    let reply: String = reply.split_whitespace().collect();
    assert_eq!(reply, r"033[30;100R");
}

// V6: a normal exit is reported once, as Exited.
#[tokio::test]
#[ignore = "needs hc; set HERDR_TEST_HC=1"]
async fn core_v6_exit_is_reported_once() {
    let Some(_core) = ScratchCore::start() else {
        return;
    };
    let scratch = Scratch::new("core-v6");
    let (runtime, mut events) = spawn_sh(
        &scratch.0,
        "sleep 0.2; exit 3",
        &[],
        &PaneLaunchEnv::default(),
        24,
        80,
        0,
    );
    assert!(is_core(&runtime));
    let died = next_pane_died(&mut events, WAIT).await;
    let again = next_pane_died(&mut events, Duration::from_millis(300)).await;
    let pane_id = runtime.pane_id;
    runtime.shutdown();
    assert_eq!(
        died,
        Some((pane_id, crate::platform::ChildExitReason::Exited))
    );
    assert_eq!(again, None);
}

// V7: closing a core pane ends its detached job and never reaches core.
#[tokio::test]
#[ignore = "needs hc; set HERDR_TEST_HC=1"]
async fn core_v7_close_ends_detached_jobs_and_spares_core() {
    let Some(core) = ScratchCore::start() else {
        return;
    };
    let scratch = Scratch::new("core-v7");
    let (other, _other_events) = spawn_sh(
        &scratch.0,
        "exec sleep 60",
        &[],
        &PaneLaunchEnv::default(),
        24,
        80,
        0,
    );
    let (runtime, _events) = spawn_sh(
        &scratch.0,
        "nohup sleep 300 >/dev/null 2>&1 & echo $! > detached; exec sleep 60",
        &[],
        &PaneLaunchEnv::default(),
        24,
        80,
        0,
    );
    assert!(is_core(&runtime) && is_core(&other));
    let detached: u32 = wait_for_file(&scratch.path("detached"), |text| text.ends_with('\n'))
        .await
        .trim()
        .parse()
        .unwrap();
    let (core_pid, holders) = core.pids();
    let other_pid = other.child_pid().expect("other pane pid");

    drop(runtime);

    assert!(wait_until_gone(detached), "the detached job ended");
    assert!(
        crate::platform::process_exists(core_pid),
        "core server alive"
    );
    for holder in holders {
        assert!(
            crate::platform::process_exists(holder),
            "holder {holder} alive"
        );
    }
    assert!(
        crate::platform::process_exists(other_pid),
        "the other pane's program alive"
    );
    other.shutdown();
}

fn last_tick(text: &str) -> Option<u64> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("tick ")?.parse().ok())
        .max()
}

// V3 (reattach, D5): a core restart closes the stream; the pane reattaches and
// keeps receiving the program's output, which never stopped.
#[tokio::test]
#[ignore = "needs hc; set HERDR_TEST_HC=1"]
async fn core_v3_reattaches_after_a_core_restart() {
    let Some(core) = ScratchCore::start() else {
        return;
    };
    let scratch = Scratch::new("core-v3r");
    let (runtime, mut events) = spawn_sh(
        &scratch.0,
        "i=0; while :; do i=$((i+1)); echo \"tick $i\"; sleep 0.1; done",
        &[],
        &PaneLaunchEnv::default(),
        24,
        80,
        0,
    );
    assert!(is_core(&runtime));
    let pid = runtime.child_pid().expect("pid");
    let before = tokio::time::timeout(WAIT, async {
        loop {
            if let Some(tick) = last_tick(&runtime.visible_text()) {
                if tick >= 3 {
                    break tick;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("ticks before the restart");

    core.restart();

    let after = tokio::time::timeout(WAIT, async {
        loop {
            if let Some(tick) = last_tick(&runtime.visible_text()) {
                if tick >= before + 10 {
                    break tick;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("ticks after the restart reach the pane");
    let died = next_pane_died(&mut events, Duration::from_millis(100)).await;
    let alive = crate::platform::process_exists(pid);
    runtime.shutdown();

    assert!(after > before);
    assert_eq!(died, None, "a core restart is not an exit");
    assert!(alive, "the program kept running across the core restart");
}

// V8, as security review M3 restated it: OSC 7501 printed in a pane (here by
// `cat` of a forged file, as a fetched page or log would be) never becomes an
// agent report or a blocked hint. core only learns status from printed bytes,
// so herdr does not use it; detection and hooks decide.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs hc; set HERDR_TEST_HC=1"]
async fn core_v8_printed_status_is_not_an_agent_report() {
    let Some(_core) = ScratchCore::start() else {
        return;
    };
    let scratch = Scratch::new("core-v8");
    std::fs::write(
        scratch.path("forged.log"),
        "\x1b]7501;state=idle:app=claude-code\x07\x1b]7501;state=blocked:kind=permission:app=claude-code\x07V8-PRINTED\n",
    )
    .expect("forged file");
    let (runtime, mut events) = spawn_sh(
        &scratch.0,
        "sleep 0.3; cat forged.log; exec sleep 30",
        &[],
        &PaneLaunchEnv::default(),
        24,
        80,
        0,
    );
    assert!(is_core(&runtime));
    assert!(
        wait_for_text(&runtime, "V8-PRINTED").await,
        "the file was printed"
    );
    // Give a status event, were one forwarded, time to arrive.
    tokio::time::sleep(Duration::from_millis(500)).await;
    runtime.shutdown();
    while let Ok(event) = events.try_recv() {
        assert!(
            !matches!(
                event,
                AppEvent::HookStateReported { .. } | AppEvent::AgentHintReported { .. }
            ),
            "printed bytes became an agent report"
        );
    }
}

// core-status-mirror verifier 12-13: the status herdr mirrors reaches core's
// record through the program's terminal and `clear` removes it, while the
// echo in the pane's stream changes nothing herdr shows or reports.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs hc; set HERDR_TEST_HC=1"]
async fn core_status_mirror_reaches_cores_record_and_clears() {
    let Some(core) = ScratchCore::start() else {
        return;
    };
    let scratch = Scratch::new("core-mirror");
    let (runtime, mut events) = spawn_sh(
        &scratch.0,
        "printf 'MIRROR-READY\\n'; exec sleep 30",
        &[],
        &PaneLaunchEnv::default(),
        24,
        80,
        0,
    );
    assert!(is_core(&runtime));
    assert!(wait_for_text(&runtime, "MIRROR-READY").await);
    let address = runtime
        .core_terminal()
        .expect("a core pane has a link")
        .address;
    assert!(
        core.status(&address).is_null(),
        "no status before the mirror"
    );
    let screen = runtime.visible_text();

    runtime.mirror_core_status(CoreStatusRecord::Report {
        word: CoreStatusWord::Working,
        app: CoreStatusApp::parse("claude"),
    });
    let status = core
        .wait_for_status(&address, |status| status["state"] == "working")
        .await;
    assert_eq!(status["app"], "claude", "{status}");

    runtime.mirror_core_status(CoreStatusRecord::Report {
        word: CoreStatusWord::Blocked,
        app: CoreStatusApp::parse("claude"),
    });
    let status = core
        .wait_for_status(&address, |status| status["state"] == "blocked")
        .await;
    assert_eq!(status["kind"], "permission", "{status}");

    runtime.mirror_core_status(CoreStatusRecord::Clear);
    core.wait_for_status(&address, serde_json::Value::is_null)
        .await;

    // Give the echo, and any event it could raise, time to arrive.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let after = runtime.visible_text();
    runtime.shutdown();
    assert_eq!(after, screen, "the echoed records change no screen text");
    while let Ok(event) = events.try_recv() {
        assert!(
            !matches!(
                event,
                AppEvent::HookStateReported { .. } | AppEvent::AgentHintReported { .. }
            ),
            "a mirrored record became an agent report"
        );
    }
}

async fn wait_for_text(runtime: &PaneRuntime, needle: &str) -> bool {
    tokio::time::timeout(WAIT, async {
        while !runtime.visible_text().contains(needle) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .is_ok()
}

fn reattach(
    link: &CoreTerminalLink,
    public_pane_id: &str,
) -> (std::io::Result<PaneRuntime>, mpsc::Receiver<AppEvent>) {
    let (events, event_rx) = mpsc::channel(64);
    let runtime = PaneRuntime::reattach_core(
        PaneId::alloc(),
        24,
        80,
        link.clone(),
        Some(public_pane_id.to_string()),
        0,
        crate::terminal_theme::TerminalTheme::default(),
        None,
        events,
        Arc::new(Notify::new()),
        Arc::new(RenderSignal::new()),
    );
    (runtime, event_rx)
}

// V10 (pane half, D10): a core pane kept for the next server outlives its
// runtime; the next runtime attaches to the same program only when the labels
// name its pane, shows its screen, types nothing into it (R6), takes input,
// and ends it when closed.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs hc; set HERDR_TEST_HC=1"]
async fn core_v10_a_kept_pane_is_reattached_by_the_next_server() {
    let Some(_core) = ScratchCore::start() else {
        return;
    };
    let scratch = Scratch::new("core-v10");
    let env = PaneLaunchEnv::from_extra(Vec::new()).with_identity(
        "w9".into(),
        "w9-1".into(),
        "w9-p1".into(),
    );
    let (runtime, _events) = spawn_sh(
        &scratch.0,
        "echo V10-READY; while IFS= read -r line; do echo \"$line\" >> in.txt; done",
        &[],
        &env,
        24,
        80,
        0,
    );
    assert!(is_core(&runtime));
    assert!(wait_for_text(&runtime, "V10-READY").await, "first screen");
    let pid = runtime.child_pid().expect("pid");
    let link = runtime.core_terminal().expect("a core pane has a link");

    // `herdr server restart`: the old server lets go without a kill.
    runtime.set_keep_core_terminal(true);
    drop(runtime);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(crate::platform::process_exists(pid), "kept program alive");

    // Unclaimed while no pane shows it: reported as an orphan, not killed.
    let unclaimed = crate::pane::unclaimed_core_terminals(&[]);
    assert!(
        unclaimed
            .iter()
            .any(|(address, pane)| *address == link.address && pane.as_deref() == Some("w9-p1")),
        "{unclaimed:?}"
    );
    assert!(
        !crate::pane::unclaimed_core_terminals(std::slice::from_ref(&link.address))
            .iter()
            .any(|(address, _)| *address == link.address),
        "a claimed terminal is not an orphan"
    );

    // Labelled for another pane: refused, and the program is left alone.
    let (refused, _) = reattach(&link, "w9-p2");
    assert!(refused.is_err(), "another pane's label is refused");
    assert!(
        crate::platform::process_exists(pid),
        "a refusal kills nothing"
    );

    let (reattached, mut events) = reattach(&link, "w9-p1");
    let reattached = reattached.expect("reattach");
    assert!(is_core(&reattached));
    assert_eq!(reattached.child_pid(), Some(pid), "the same program");
    assert!(
        wait_for_text(&reattached, "V10-READY").await,
        "the first paint shows the earlier screen: {:?}",
        reattached.visible_text()
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !scratch.path("in.txt").exists(),
        "nothing was typed into the running program (R6)"
    );

    reattached
        .try_send_bytes(Bytes::from_static(b"after-reattach\r"))
        .expect("input");
    let input = wait_for_file(&scratch.path("in.txt"), |text| text.ends_with('\n')).await;
    assert_eq!(input, "after-reattach\n");
    assert_eq!(
        next_pane_died(&mut events, Duration::from_millis(100)).await,
        None
    );

    // A pane close ends it (D8).
    drop(reattached);
    assert!(wait_until_gone(pid), "closing the reattached pane ends it");
}

// D1: with `runtime = "core"`, a pane restored fresh (no saved core terminal,
// or one that is gone) still runs on the PTY; only new panes use core.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs hc; set HERDR_TEST_HC=1"]
async fn core_v1_a_restored_pane_stays_on_the_pty() {
    let Some(_core) = ScratchCore::start() else {
        return;
    };
    let scratch = Scratch::new("core-v1r");
    let (events, _event_rx) = mpsc::channel(64);
    let restored = PaneRuntime::spawn_restored(
        PaneId::alloc(),
        24,
        80,
        scratch.0.clone(),
        0,
        crate::terminal_theme::TerminalTheme::default(),
        None,
        PaneShellConfig::new("/bin/sh", crate::config::ShellModeConfig::default()),
        &PaneLaunchEnv::default(),
        Some("saved history\r\n"),
        events,
        Arc::new(Notify::new()),
        Arc::new(RenderSignal::new()),
    )
    .expect("restored pane");
    assert!(!is_core(&restored), "a restored pane stays on the PTY");
    assert!(restored.core_terminal().is_none());
    restored.shutdown();
}

// V3 (flow, D3): output far beyond core's 256 KiB credit window arrives whole
// with credit sent back in batches; a stall would leave the marker unseen.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs hc; set HERDR_TEST_HC=1"]
async fn core_v3_a_burst_beyond_the_credit_window_arrives() {
    let Some(_core) = ScratchCore::start() else {
        return;
    };
    let scratch = Scratch::new("core-v3b");
    let (runtime, _events) = spawn_sh(
        &scratch.0,
        "head -c 2000000 /dev/zero | tr '\\0' 'x' | fold -w 100; echo; echo V3-BURST-DONE; exec sleep 30",
        &[],
        &PaneLaunchEnv::default(),
        24,
        100,
        0,
    );
    assert!(is_core(&runtime));
    assert!(
        wait_for_text(&runtime, "V3-BURST-DONE").await,
        "the end of a 2 MB burst reached the pane: {:?}",
        runtime.visible_text()
    );
    runtime.shutdown();
}

/// The `herdr-app` document as core holds it: `(value, version)`.
async fn core_doc(expect: impl Fn(&serde_json::Value) -> bool) -> (serde_json::Value, u64) {
    let read = || {
        let reply =
            runtime_io::core_call_for_test("doc.get", serde_json::json!({"name": "herdr-app"}))
                .expect("doc.get");
        (
            reply["value"].clone(),
            reply["version"].as_u64().expect("version"),
        )
    };
    tokio::time::timeout(WAIT, async {
        loop {
            let doc = read();
            if expect(&doc.0) {
                return doc;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("herdr-app document: last {:?}", read()))
}

// core-spaces-doc verifier 10-11: the publisher merges herdr-max's spaces
// into core's document beside a foreign space, merges again after a foreign
// write, and withdraws only its own.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs hc; set HERDR_TEST_HC=1"]
async fn core_spaces_doc_keeps_foreign_spaces_and_withdraws_its_own() {
    let Some(_core) = ScratchCore::start() else {
        return;
    };
    let scratch = Scratch::new("core-spaces");
    let (runtime, _events) = spawn_sh(
        &scratch.0,
        "exec sleep 30",
        &[],
        &PaneLaunchEnv::default(),
        24,
        80,
        0,
    );
    assert!(is_core(&runtime));
    let address = runtime
        .core_terminal()
        .expect("a core pane has a link")
        .address;
    let ours = |name: &str| {
        serde_json::json!({"id": "w1", "name": name, "folder": "/work", "tabs": [
            {"id": "w1:t1", "name": "1", "root": {"id": "w1:t1", "type": "row", "children": [
                {"id": "w1:p1", "type": "block", "content": {"terminal": address}}]}}]})
    };
    let marked = |name: &str| {
        let mut space = ours(name);
        space["herdr_max"] = serde_json::json!(1);
        space
    };
    let phone = serde_json::json!({"id": "s9", "name": "phone", "folder": "/p", "color": "teal", "tabs": []});
    let doc_set = |value: serde_json::Value, version: u64| {
        runtime_io::core_call_for_test(
            "doc.set",
            serde_json::json!({"name": "herdr-app", "value": value, "version": version}),
        )
        .expect("a foreign write")["version"]
            .as_u64()
            .expect("version")
    };
    let (_, version) = core_doc(|_| true).await;
    doc_set(serde_json::json!({"spaces": [phone]}), version);

    runtime_io::publish_core_spaces(vec![ours("first")]);
    let (_, version) =
        core_doc(|value| value == &serde_json::json!({"spaces": [phone, marked("first")]})).await;

    let second_phone = serde_json::json!({"id": "s8", "name": "mac", "folder": "/m", "tabs": []});
    doc_set(
        serde_json::json!({"spaces": [marked("first"), phone, second_phone]}),
        version,
    );
    runtime_io::publish_core_spaces(vec![ours("second")]);
    core_doc(|value| {
        value == &serde_json::json!({"spaces": [marked("second"), phone, second_phone]})
    })
    .await;

    runtime_io::publish_core_spaces(Vec::new());
    core_doc(|value| value == &serde_json::json!({"spaces": [phone, second_phone]})).await;
    runtime.shutdown();
}
