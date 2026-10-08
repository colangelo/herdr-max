//! Runs the real Claude hook script against a fake herdr socket and checks the
//! resume command it reports (fork issue 123).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// Written after any process the tests start: what the live process wrote.
const LATER: &str = "2099-01-01T00:00:00.000Z";
/// Written before it: what an earlier run of the session wrote.
const EARLIER: &str = "2020-01-01T00:00:00.000Z";
const SESSION: &str = "4f1c2d3e-aaaa-bbbb-cccc-0123456789ab";

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // Short: a unix socket path must stay under ~100 bytes.
        let dir = PathBuf::from(format!("/tmp/hch-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Runs the hook with `input` on stdin. With `claude_flags`, the hook runs as
/// the child of a process named `claude` launched with those flags, the way
/// Claude Code runs its hooks. Returns the request the hook sent, if any.
fn run_hook(input: &Value, claude_flags: Option<&[&str]>) -> Option<Value> {
    run_hook_with_env(input, claude_flags, &[])
}

/// [`run_hook`] with extra environment variables, as Claude sets them for its
/// hooks.
fn run_hook_with_env(
    input: &Value,
    claude_flags: Option<&[&str]>,
    envs: &[(&str, &str)],
) -> Option<Value> {
    let scratch = Scratch::new();
    let socket_path = scratch.0.join("s");
    let listener = UnixListener::bind(&socket_path).unwrap();
    let server = std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    let mut line = String::new();
                    BufReader::new(stream.try_clone().unwrap())
                        .read_line(&mut line)
                        .unwrap();
                    let _ = stream.write_all(b"{\"id\":\"t\",\"result\":{\"type\":\"ok\"}}\n");
                    return Some(line);
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(err) => panic!("accept failed: {err}"),
            }
        }
        None
    });

    let hook = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/integration/assets/claude/herdr-agent-state.sh");
    let mut command = Command::new("bash");
    match claude_flags {
        Some(flags) => {
            command
                .arg("-c")
                .arg(concat!(
                    "exec -a claude bash -c ",
                    r#"'bash "$HERDR_TEST_HOOK" session; exit $?' "#,
                    "claude $HERDR_TEST_CLAUDE_FLAGS",
                ))
                .env("HERDR_TEST_HOOK", &hook)
                .env("HERDR_TEST_CLAUDE_FLAGS", flags.join(" "));
        }
        None => {
            command.arg(&hook).arg("session");
        }
    }
    let mut child = command
        .env("HERDR_ENV", "1")
        .env("HERDR_SOCKET_PATH", &socket_path)
        .env("HERDR_PANE_ID", "p_test")
        .envs(envs.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "hook failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let line = server.join().unwrap()?;
    Some(serde_json::from_str(&line).unwrap())
}

fn user_line(mode: &str) -> Value {
    json!({"type": "user", "permissionMode": mode, "timestamp": LATER, "message": {"role": "user", "content": "hi"}})
}

fn assistant_line(model: &str, effort: &str) -> Value {
    json!({"type": "assistant", "effort": effort, "timestamp": LATER, "message": {"role": "assistant", "model": model}})
}

/// The resume command the hook reports for `event` with `extra` hook input
/// fields over a transcript made of `lines`.
fn resume_argv(
    event: &str,
    extra: Value,
    lines: &[Value],
    claude_flags: Option<&[&str]>,
) -> Option<Vec<String>> {
    let scratch = Scratch::new();
    let transcript = scratch.0.join("t.jsonl");
    let body: String = lines.iter().map(|line| format!("{line}\n")).collect();
    std::fs::write(&transcript, body).unwrap();
    let mut input = json!({
        "hook_event_name": event,
        "session_id": SESSION,
        "transcript_path": transcript.display().to_string(),
    });
    for (key, value) in extra.as_object().unwrap() {
        input[key] = value.clone();
    }
    let request = run_hook(&input, claude_flags).expect("the hook reports the session");
    assert_eq!(request["method"], "pane.report_agent_session");
    request["params"]["resume_argv"].as_array().map(|argv| {
        argv.iter()
            .map(|arg| arg.as_str().unwrap().to_string())
            .collect()
    })
}

#[test]
fn resume_keeps_auto_and_the_bypass_the_pane_was_launched_with() {
    let argv = resume_argv(
        "Stop",
        json!({}),
        &[
            user_line("auto"),
            assistant_line("claude-sonnet-5-5", "low"),
        ],
        Some(&["--dangerously-skip-permissions"]),
    );
    assert_eq!(
        argv.unwrap(),
        [
            "claude",
            "--resume",
            SESSION,
            "--model",
            "claude-sonnet-5-5",
            "--effort",
            "low",
            "--allow-dangerously-skip-permissions",
            "--permission-mode",
            "auto",
        ]
    );
}

#[test]
fn resume_in_bypass_keeps_bypass() {
    let argv = resume_argv(
        "UserPromptSubmit",
        json!({"permission_mode": "bypassPermissions"}),
        &[],
        None,
    );
    assert_eq!(
        argv.unwrap(),
        [
            "claude",
            "--resume",
            SESSION,
            "--allow-dangerously-skip-permissions",
            "--permission-mode",
            "bypassPermissions",
        ]
    );
}

#[test]
fn resume_with_nothing_known_is_plain() {
    let argv = resume_argv("SessionStart", json!({"source": "startup"}), &[], None);
    assert_eq!(argv.unwrap(), ["claude", "--resume", SESSION]);
}

#[test]
fn resume_takes_the_mode_from_the_launch_before_any_prompt() {
    let argv = resume_argv(
        "SessionStart",
        json!({"source": "startup", "model": "claude-opus-5-5"}),
        &[],
        Some(&["--permission-mode", "plan"]),
    );
    assert_eq!(
        argv.unwrap(),
        [
            "claude",
            "--resume",
            SESSION,
            "--model",
            "claude-opus-5-5",
            "--permission-mode",
            "plan",
        ]
    );
}

#[test]
fn resume_prefers_the_live_mode_and_model() {
    let argv = resume_argv(
        "UserPromptSubmit",
        json!({"permission_mode": "plan", "model": "claude-opus-5-5"}),
        &[
            user_line("acceptEdits"),
            assistant_line("claude-sonnet-5-5", "high"),
        ],
        None,
    );
    assert_eq!(
        argv.unwrap(),
        [
            "claude",
            "--resume",
            SESSION,
            "--model",
            "claude-opus-5-5",
            "--effort",
            "high",
            "--permission-mode",
            "plan",
        ]
    );
}

#[test]
fn resume_skips_synthetic_models_and_remembers_bypass() {
    let argv = resume_argv(
        "Stop",
        json!({}),
        &[
            user_line("bypassPermissions"),
            assistant_line("claude-opus-5-5", "xhigh"),
            user_line("default"),
            assistant_line("<synthetic>", "xhigh"),
        ],
        None,
    );
    assert_eq!(
        argv.unwrap(),
        [
            "claude",
            "--resume",
            SESSION,
            "--model",
            "claude-opus-5-5",
            "--effort",
            "xhigh",
            "--allow-dangerously-skip-permissions",
            "--permission-mode",
            "default",
        ]
    );
}

#[test]
fn resume_ignores_values_claude_would_not_take() {
    let argv = resume_argv(
        "UserPromptSubmit",
        json!({"permission_mode": "yolo", "model": "bad model"}),
        &[assistant_line("claude-opus-5-5", "turbo")],
        None,
    );
    assert_eq!(
        argv.unwrap(),
        ["claude", "--resume", SESSION, "--model", "claude-opus-5-5"]
    );
}

#[test]
fn resume_is_left_out_when_the_session_id_would_break_it() {
    let input = json!({"hook_event_name": "Stop", "session_id": "it's"});
    let request = run_hook(&input, None).expect("the hook still reports the session");
    assert!(request["params"].get("resume_argv").is_none());
}

#[test]
fn prompt_and_stop_reports_carry_no_session_start_source() {
    for event in ["UserPromptSubmit", "Stop"] {
        let input = json!({"hook_event_name": event, "session_id": SESSION, "source": "startup"});
        let request = run_hook(&input, None).expect("the hook reports the session");
        assert!(
            request["params"].get("session_start_source").is_none(),
            "{event}"
        );
    }
}

/// Fork issue 143: a `claude -p` started from a session's Bash tool inherits
/// `HERDR_PANE_ID`. Its hooks must not report its own session to the pane the
/// parent owns; a top-level session still does.
#[test]
fn a_nested_print_mode_claude_does_not_take_the_panes_session() {
    let input =
        json!({"hook_event_name": "SessionStart", "session_id": SESSION, "source": "startup"});

    // Claude sets this for a `-p` run, in the hook's environment.
    assert!(
        run_hook_with_env(&input, None, &[("CLAUDE_CODE_ENTRYPOINT", "sdk-cli")]).is_none(),
        "the entrypoint of a print-mode run is not the pane's agent"
    );
    // And the flags it was started with say so too.
    assert!(run_hook(&input, Some(&["-p", "say", "ok"])).is_none(), "-p");
    assert!(
        run_hook(&input, Some(&["--print", "say", "ok"])).is_none(),
        "--print"
    );
}

#[test]
fn a_top_level_interactive_claude_still_reports_its_session() {
    let input =
        json!({"hook_event_name": "SessionStart", "session_id": SESSION, "source": "startup"});

    let request = run_hook_with_env(&input, Some(&[]), &[("CLAUDE_CODE_ENTRYPOINT", "cli")])
        .expect("an interactive session reports");

    assert_eq!(request["params"]["agent_session_id"], SESSION);
}

fn at(mut line: Value, timestamp: &str) -> Value {
    line["timestamp"] = json!(timestamp);
    line
}

/// Fork issue 135: a session resumed by id with new `--model`/`--effort` has a
/// transcript whose last records are the earlier process's. The hook must not
/// report those; the flags the live process started with are newer.
#[test]
fn a_resume_by_id_reports_the_new_flags_not_the_earlier_runs_records() {
    let earlier = [
        at(user_line("default"), EARLIER),
        at(assistant_line("claude-opus-5-5", "high"), EARLIER),
    ];
    let argv = resume_argv(
        "SessionStart",
        json!({"source": "resume"}),
        &earlier,
        Some(&["--model", "claude-sonnet-5-5", "--effort", "medium"]),
    )
    .unwrap();
    assert_eq!(
        argv,
        [
            "claude",
            "--resume",
            SESSION,
            "--model",
            "claude-sonnet-5-5",
            "--effort",
            "medium"
        ],
        "the new flags win over the earlier run's opus/high"
    );
}

#[test]
fn an_earlier_runs_model_and_effort_are_not_reported_for_a_flagless_resume() {
    let earlier = [
        at(user_line("default"), EARLIER),
        at(assistant_line("claude-opus-5-5", "high"), EARLIER),
    ];
    let argv = resume_argv(
        "SessionStart",
        json!({"source": "resume"}),
        &earlier,
        Some(&[]),
    );
    assert_eq!(
        argv.unwrap(),
        ["claude", "--resume", SESSION],
        "the process runs its defaults, not the earlier run's"
    );
}

#[test]
fn what_the_live_process_wrote_beats_its_launch_flags() {
    let lines = [
        at(assistant_line("claude-opus-5-5", "high"), EARLIER),
        at(assistant_line("claude-haiku-4-5", "low"), LATER),
    ];
    let argv = resume_argv(
        "Stop",
        json!({}),
        &lines,
        Some(&["--model", "claude-sonnet-5-5", "--effort", "medium"]),
    )
    .unwrap();
    assert_eq!(
        argv,
        [
            "claude",
            "--resume",
            SESSION,
            "--model",
            "claude-haiku-4-5",
            "--effort",
            "low"
        ],
        "a /model change in the session is newer than the launch flag"
    );
}
