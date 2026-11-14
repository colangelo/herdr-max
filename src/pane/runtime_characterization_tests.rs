//! Characterization of the PTY-backed pane runtime before the backend seam
//! (openspec/changes/runtime-seam, design.md § Verifier). Each test pins what a
//! pane does today, so moving the PTY code behind `PaneIo` cannot change it.
//! Every program runs under `/bin/sh` directly, so the user's shell cannot
//! change a result.

use super::runtime_io::{PaneIoSink, ReadCallback, SpawnSpec};
use super::*;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub(super) const WAIT: Duration = Duration::from_secs(10);

pub(super) struct Scratch(pub(super) PathBuf);

impl Scratch {
    pub(super) fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("herdr-seam-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // The program sees the resolved path (macOS /var is /private/var).
        Self(dir.canonicalize().unwrap())
    }

    pub(super) fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(super) fn spawn_sh(
    cwd: &Path,
    script: &str,
    extra_args: &[&str],
    env: &PaneLaunchEnv,
    rows: u16,
    cols: u16,
    scrollback_limit_bytes: usize,
) -> (PaneRuntime, mpsc::Receiver<AppEvent>) {
    let (events, event_rx) = mpsc::channel(64);
    let mut argv = vec!["/bin/sh".to_string(), "-c".to_string(), script.to_string()];
    argv.extend(extra_args.iter().map(|arg| (*arg).to_string()));
    let runtime = PaneRuntime::spawn_argv_command(
        PaneId::alloc(),
        rows,
        cols,
        cwd.to_path_buf(),
        &argv,
        env,
        AgentDetection::Disabled,
        scrollback_limit_bytes,
        crate::terminal_theme::TerminalTheme::default(),
        None,
        events,
        Arc::new(Notify::new()),
        Arc::new(RenderSignal::new()),
    )
    .expect("spawn /bin/sh pane");
    (runtime, event_rx)
}

pub(super) async fn wait_for_file(path: &Path, done: impl Fn(&str) -> bool) -> String {
    tokio::time::timeout(WAIT, async {
        loop {
            if let Ok(text) = std::fs::read_to_string(path) {
                if done(&text) {
                    break text;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{} never completed", path.display()))
}

pub(super) async fn next_pane_died(
    event_rx: &mut mpsc::Receiver<AppEvent>,
    within: Duration,
) -> Option<(PaneId, crate::platform::ChildExitReason)> {
    tokio::time::timeout(within, async {
        loop {
            match event_rx.recv().await {
                Some(AppEvent::PaneDied {
                    pane_id,
                    exit_reason,
                }) => break Some((pane_id, exit_reason)),
                Some(_) => continue,
                None => break None,
            }
        }
    })
    .await
    .ok()
    .flatten()
}

pub(super) fn wait_until_gone(pid: u32) -> bool {
    let deadline = Instant::now() + WAIT;
    while crate::platform::process_exists(pid) {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

// V1: argv, cwd and an environment variable reach the program.
#[tokio::test]
async fn v1_spawn_passes_argv_cwd_and_env_to_the_program() {
    let scratch = Scratch::new("v1");
    let env = PaneLaunchEnv::from_extra(vec![("HERDR_SEAM_PROBE".into(), "v1-probe".into())]);
    let (runtime, _events) = spawn_sh(
        &scratch.0,
        r#"printf '%s|%s|%s\n' "$0" "$(pwd -P)" "$HERDR_SEAM_PROBE" > out.txt"#,
        &["seam-arg0"],
        &env,
        24,
        80,
        0,
    );

    let out = wait_for_file(&scratch.path("out.txt"), |text| text.ends_with('\n')).await;
    runtime.shutdown();

    assert_eq!(out, format!("seam-arg0|{}|v1-probe\n", scratch.0.display()));
}

// V8 (pane level) and roundtable F1: the shell pid is published by the time
// spawn returns, and it is the program's own pid.
#[tokio::test]
async fn v8_shell_pid_is_published_when_spawn_returns() {
    let scratch = Scratch::new("v8");
    let (runtime, _events) = spawn_sh(
        &scratch.0,
        "echo $$ > pid.txt; exec sleep 30",
        &[],
        &PaneLaunchEnv::default(),
        24,
        80,
        0,
    );
    let published = runtime.child_pid();

    let reported = wait_for_file(&scratch.path("pid.txt"), |text| text.ends_with('\n')).await;
    runtime.shutdown();

    assert_eq!(published, reported.trim().parse::<u32>().ok());
}

// V2: one PaneDied per pane, classified as today, after the wait is marked
// complete.
#[tokio::test]
async fn v2_exit_is_reported_once_as_exited_after_the_wait_completes() {
    let scratch = Scratch::new("v2-exit");
    let (runtime, mut events) = spawn_sh(
        &scratch.0,
        "exit 3",
        &[],
        &PaneLaunchEnv::default(),
        24,
        80,
        0,
    );

    let died = next_pane_died(&mut events, WAIT).await;
    let wait_marked = runtime
        .child_wait_completed
        .as_ref()
        .is_some_and(|flag| flag.load(Ordering::Acquire));
    let again = next_pane_died(&mut events, Duration::from_millis(300)).await;
    let pane_id = runtime.pane_id;
    runtime.shutdown();

    assert_eq!(
        died,
        Some((pane_id, crate::platform::ChildExitReason::Exited))
    );
    assert!(wait_marked, "the wait is marked complete before PaneDied");
    assert_eq!(again, None, "PaneDied is sent once");
}

#[tokio::test]
async fn v2_signal_death_is_reported_as_interrupted() {
    let scratch = Scratch::new("v2-signal");
    let (runtime, mut events) = spawn_sh(
        &scratch.0,
        "kill -KILL $$",
        &[],
        &PaneLaunchEnv::default(),
        24,
        80,
        0,
    );

    let died = next_pane_died(&mut events, WAIT).await;
    let pane_id = runtime.pane_id;
    runtime.shutdown();

    assert_eq!(
        died,
        Some((pane_id, crate::platform::ChildExitReason::Interrupted))
    );
}

// V2: a pane adopted from a handoff fd has no wait handle; it reports Handoff
// once, when its reader ends.
#[tokio::test]
async fn v2_adopted_handoff_pane_reports_handoff_once_when_its_reader_ends() {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");
    let owned = runtime_io::test_dup_master_fd(&pair);
    let pane_id = PaneId::alloc();
    let state = crate::handoff_runtime::HandoffRuntimeState {
        pane_id: pane_id.raw(),
        // No process to signal: this pane only proves the exit report.
        child_pid: 0,
        rows: 24,
        cols: 80,
        cell_width_px: 10,
        cell_height_px: 20,
        keyboard_protocol_flags: 0,
        keyboard_protocol_ansi: None,
        input_state: None,
        terminal_title: None,
        initial_history_ansi: None,
        agent: None,
        agent_state: None,
        hook_agent_state: None,
        unseen: false,
    };
    let (events, mut event_rx) = mpsc::channel(64);
    let runtime = PaneRuntime::from_handoff_fd(
        crate::handoff_runtime::ImportedHandoffRuntime {
            master_fd: owned,
            state,
        },
        1024 * 1024,
        crate::terminal_theme::TerminalTheme::default(),
        None,
        events,
        Arc::new(Notify::new()),
        Arc::new(RenderSignal::new()),
    )
    .expect("import handoff pane");
    // An adopted reader starts quiesced, as after a handoff commit.
    runtime.set_handoff_reader_paused(false);
    drop(pair);

    let died = next_pane_died(&mut event_rx, WAIT).await;
    let again = next_pane_died(&mut event_rx, Duration::from_millis(300)).await;
    drop(runtime);

    assert_eq!(
        died,
        Some((pane_id, crate::platform::ChildExitReason::Handoff))
    );
    assert_eq!(again, None, "PaneDied is sent once");
}

pub(super) const SIGNAL_LOGGER: &str = r#"trap 'echo HUP >> sig.log' HUP
trap 'echo TERM >> sig.log' TERM
echo $$ > ready
while :; do sleep 0.05; done"#;

const HUP_EXITS: &str = r#"trap 'echo HUP >> sig.log; exit 0' HUP
trap 'echo TERM >> sig.log' TERM
echo $$ > ready
while :; do sleep 0.05; done"#;

// V3: dropping a pane signals its session HUP, then TERM, then KILL. The time
// is logged, not asserted (AGENTS.md: no wall-clock limits in CI).
#[tokio::test]
async fn v3_drop_escalates_hangup_terminate_kill_in_order() {
    let scratch = Scratch::new("v3");
    let (runtime, _events) = spawn_sh(
        &scratch.0,
        SIGNAL_LOGGER,
        &[],
        &PaneLaunchEnv::default(),
        24,
        80,
        0,
    );
    let pid: u32 = wait_for_file(&scratch.path("ready"), |text| text.ends_with('\n'))
        .await
        .trim()
        .parse()
        .unwrap();

    let started = Instant::now();
    drop(runtime);
    eprintln!("v3: drop returned after {:?}", started.elapsed());

    assert!(wait_until_gone(pid), "the program is gone after the drop");
    let log = std::fs::read_to_string(scratch.path("sig.log")).unwrap_or_default();
    assert_eq!(log, "HUP\nTERM\n");
}

// The first hangup here comes from the kernel: `Drop` shuts the reader down,
// which closes the master fd before the sweep starts. So this pins the end
// state a pane close gives today, not the sweep's own order (the test above
// does that). A backend that does not own the master fd (herdr-core) must
// still deliver this hangup.
#[tokio::test]
async fn v3_a_program_that_exits_on_hangup_gets_only_hangup() {
    let scratch = Scratch::new("v3-hup");
    let (runtime, _events) = spawn_sh(
        &scratch.0,
        HUP_EXITS,
        &[],
        &PaneLaunchEnv::default(),
        24,
        80,
        0,
    );
    let pid: u32 = wait_for_file(&scratch.path("ready"), |text| text.ends_with('\n'))
        .await
        .trim()
        .parse()
        .unwrap();

    drop(runtime);

    assert!(wait_until_gone(pid), "the program is gone after the drop");
    let log = std::fs::read_to_string(scratch.path("sig.log")).unwrap_or_default();
    assert_eq!(log, "HUP\n");
}

// V4: a pane preserved for a handoff sends no signal when it is dropped. The
// test holds a duplicate of the master fd, as the replacement server does, so
// the terminal is not hung up by the kernel either.
#[tokio::test]
async fn v4_preserved_pane_sends_no_signal_on_drop() {
    let scratch = Scratch::new("v4");
    let (runtime, _events) = spawn_sh(
        &scratch.0,
        SIGNAL_LOGGER,
        &[],
        &PaneLaunchEnv::default(),
        24,
        80,
        0,
    );
    let pid: u32 = wait_for_file(&scratch.path("ready"), |text| text.ends_with('\n'))
        .await
        .trim()
        .parse()
        .unwrap();

    runtime.set_handoff_reader_paused(true);
    let held_fd = runtime.duplicate_handoff_fd().expect("dup master fd");
    runtime.preserve_for_handoff();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let alive = crate::platform::process_exists(pid);
    let log = std::fs::read_to_string(scratch.path("sig.log")).unwrap_or_default();
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
        libc::close(held_fd);
    }

    assert!(alive, "a preserved program keeps running");
    assert_eq!(log, "", "a preserved program gets no signal");
}

// V5: bytes reach the parser in order. The scrollback holds every line
// (roundtable F5), so the check reads the whole run.
#[tokio::test]
async fn v5_output_reaches_the_parser_in_order() {
    const LINES: usize = 20_000;
    let scratch = Scratch::new("v5");
    let (runtime, _events) = spawn_sh(
        &scratch.0,
        "i=1; while [ $i -le 20000 ]; do echo $i; i=$((i+1)); done; echo SEAM-DONE; exec sleep 30",
        &[],
        &PaneLaunchEnv::default(),
        24,
        80,
        16 * 1024 * 1024,
    );

    let text = tokio::time::timeout(WAIT, async {
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
    assert_eq!(numbers.first(), Some(&1), "the first line is kept");
    assert_eq!(numbers, (1..=LINES).collect::<Vec<_>>());
}

// V6: a resize reaches the program, and a cursor-position query asked after it
// is answered with the new geometry.
#[tokio::test]
async fn v6_resize_reaches_the_program_and_queries_see_the_new_size() {
    let scratch = Scratch::new("v6");
    let (runtime, _events) = spawn_sh(
        &scratch.0,
        r#"echo ready > ready
while [ ! -e go ]; do sleep 0.02; done
stty size > size.txt
stty raw -echo
printf '\033[999;999H\033[6n'
dd bs=1 count=9 2>/dev/null | od -An -c > reply.txt
exec sleep 30"#,
        &[],
        &PaneLaunchEnv::default(),
        24,
        80,
        0,
    );
    wait_for_file(&scratch.path("ready"), |text| text.ends_with('\n')).await;

    runtime.resize(30, 100, 0, 0);
    std::fs::write(scratch.path("go"), "").unwrap();

    let size = wait_for_file(&scratch.path("size.txt"), |text| text.ends_with('\n')).await;
    let reply = wait_for_file(&scratch.path("reply.txt"), |text| text.contains('R')).await;
    runtime.shutdown();

    assert_eq!(size, "30 100\n");
    let reply: String = reply.split_whitespace().collect();
    assert_eq!(reply, r"033[30;100R");
}

// V14: plain writes, then a queued submission, reach the program in order:
// text, then Enter after the delay.
#[tokio::test]
async fn v14_input_and_queued_submission_arrive_in_order() {
    let scratch = Scratch::new("v14");
    let (runtime, _events) = spawn_sh(
        &scratch.0,
        r#"stty raw -echo
echo ready > ready
dd bs=1 count=5 2>/dev/null > in.bin
exec sleep 30"#,
        &[],
        &PaneLaunchEnv::default(),
        24,
        80,
        0,
    );
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

fn sh_command(script: &str) -> CommandBuilder {
    let mut command = CommandBuilder::new("/bin/sh");
    command.arg("-c");
    command.arg(script);
    command
}

fn ignore_output() -> ReadCallback {
    Box::new(|_| PtyReadResult {
        terminal_responses: Vec::new(),
    })
}

// V2, driven at the seam: the wait is marked complete before `on_exit` runs,
// seen from inside the callback (a pane-level read after PaneDied cannot tell
// the two orders apart).
#[tokio::test]
async fn v2_on_exit_runs_after_the_wait_is_marked_complete() {
    let child_pid = AtomicU32::new(0);
    let wait_completed = Arc::new(AtomicBool::new(false));
    let (seen_tx, seen_rx) = std::sync::mpsc::channel();
    let flag = Arc::clone(&wait_completed);
    let io = PaneIo::spawn(
        PaneId::alloc(),
        SpawnSpec {
            rows: 24,
            cols: 80,
            command: sh_command("exit 3"),
            spawn_error_message: "test spawn failed",
        },
        PaneIoSink {
            on_read: ignore_output(),
            on_exit: Box::new(move |reason| {
                let _ = seen_tx.send((flag.load(Ordering::Acquire), reason));
            }),
            on_status: None,
        },
        &child_pid,
        Arc::clone(&wait_completed),
    )
    .expect("spawn");

    let seen = seen_rx.recv_timeout(WAIT).expect("on_exit ran");
    io.shutdown();

    assert_eq!(seen, (true, crate::platform::ChildExitReason::Exited));
}
