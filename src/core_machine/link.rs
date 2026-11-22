//! `herdr core-link <name> <line>`: the command core runs for every dial of
//! a machine herdr added (core-phase5 D2; prototype `probe/fix/corelink.py`).
//!
//! The order is the design's: the line kind, the gate and leases, the
//! single-flight `dial.lock`, then a watcher is forked and this process
//! **execs ssh**, so the pid core holds (and SIGKILLs after about 19.8 s) is
//! ssh itself and nothing can be orphaned (F2). The watcher holds none of
//! core's streams, only the read end of ssh's stderr; it records the
//! failure classes in `gate.json` and ends a dial that is not up in 15 s.
//!
//! Leases (D3) and the retry budget (D2 step 9) come with PR B: until then
//! every machine counts as leased (task A.3), and the watcher does not
//! re-read leases (step 7).

use std::fs::OpenOptions;
use std::os::fd::{AsRawFd as _, OwnedFd};
use std::os::unix::fs::OpenOptionsExt as _;
use std::time::{Duration, Instant};

use super::name::MachineName;
use super::ssh;
use super::state::{now_unix, DialRecord, Gate, GateClass, MachineDir};
use crate::platform::core_link as sys;

/// Every refusal exits `EX_TEMPFAIL`: core retries later, locally (P4).
pub(crate) const EXIT_REFUSED: i32 = 75;
const EXIT_USAGE: i32 = 64;
/// How long a dial waits for another to release `dial.lock` (D2 step 3).
const LOCK_WAIT: Duration = Duration::from_secs(2);
/// No `HERDR-LINK-UP` by then: `auth_stalled`, before core's own kill at
/// about 19.8 s (D2 step 6, R2).
pub(crate) const UP_DEADLINE: Duration = Duration::from_secs(15);
/// Between SIGTERM and SIGKILL when the watcher ends ssh.
const TERM_GRACE: Duration = Duration::from_secs(1);

/// Why a dial was refused before any process or socket was opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Core's Windows fallback line, or any line that is not `sh -c '...'`.
    NotPosixLine,
    /// A recorded failure closes the gate (D10).
    Gate(GateClass),
    /// No lease in force (D3; PR B).
    Dormant,
}

impl Refusal {
    fn describe(&self) -> String {
        match self {
            Self::NotPosixLine => "not core's POSIX line (far sides are macOS and Linux)".into(),
            Self::Gate(class) => format!(
                "the gate is closed ({}); run herdr machine login",
                class.as_str()
            ),
            Self::Dormant => "no lease in force".into(),
        }
    }
}

/// D2 steps 1 and 2, decided from the files alone.
pub(crate) fn admit(
    line: &str,
    gate: Option<&Gate>,
    leased: bool,
    now: f64,
) -> Result<(), Refusal> {
    if !ssh::is_posix_line(line) {
        return Err(Refusal::NotPosixLine);
    }
    if let Some(gate) = gate.filter(|gate| gate.closes_at(now)) {
        return Err(Refusal::Gate(gate.class));
    }
    if !leased {
        return Err(Refusal::Dormant);
    }
    Ok(())
}

/// Whether a lease is in force. PR B (task B.1) reads `leases/`; until then
/// every machine counts as leased (task A.3), so the link behaves as core's
/// own machines do, behind the gate.
fn lease_in_force(_dir: &MachineDir) -> bool {
    true
}

/// A short, loggable picture of core's line: its kind and its end.
fn line_shape(line: &str) -> String {
    let head: String = line.chars().take(7).collect();
    let tail: String = {
        let chars: Vec<char> = line.chars().collect();
        chars[chars.len().saturating_sub(12)..].iter().collect()
    };
    format!("{head:?}..{tail:?}")
}

/// Runs `herdr core-link <name> [...] <line>`; core appends its line as the
/// last argument. Returns only when no ssh was executed.
pub(crate) fn run(args: &[String]) -> i32 {
    let started = Instant::now();
    let [name, .., line] = args else {
        eprintln!("usage: herdr core-link <machine> <line>");
        return EXIT_USAGE;
    };
    let name = match MachineName::parse(name) {
        Ok(name) => name,
        Err(err) => {
            eprintln!("herdr-link: {err}");
            return EXIT_USAGE;
        }
    };
    let account = match sys::passwd_account() {
        Ok(account) => account,
        Err(err) => {
            eprintln!("herdr-link: no passwd entry: {err}");
            return EXIT_REFUSED;
        }
    };
    let dir = MachineDir::at(&super::state::machines_root(&account.home), &name);
    if let Err(err) = dir.check() {
        eprintln!("herdr-link: {name} is not a herdr core machine here: {err}");
        return EXIT_REFUSED;
    }
    dir.log(&format!("START line={}", line_shape(line)));

    let gate = match dir.read_gate() {
        Ok(gate) => gate,
        Err(err) => {
            // An unreadable gate is a closed one.
            dir.log(&format!("REFUSE unreadable gate: {err}"));
            eprintln!("herdr-link: {name}: unreadable gate.json; run herdr machine login {name}");
            return EXIT_REFUSED;
        }
    };
    if let Err(refusal) = admit(line, gate.as_ref(), lease_in_force(&dir), now_unix()) {
        dir.log(&format!("REFUSE {refusal:?}"));
        eprintln!("herdr-link: {name}: {}", refusal.describe());
        return EXIT_REFUSED;
    }
    let target = match dir.read_target() {
        Ok(target) => target,
        Err(err) => {
            dir.log(&format!("REFUSE target: {err}"));
            eprintln!("herdr-link: {name}: {err}");
            return EXIT_REFUSED;
        }
    };

    // D2 step 3: one dial at a time. ssh inherits the descriptor, so the
    // lock lasts exactly as long as the link.
    let lock = match OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(dir.dial_lock_path())
    {
        Ok(lock) => lock,
        Err(err) => {
            dir.log(&format!("REFUSE dial.lock: {err}"));
            return EXIT_REFUSED;
        }
    };
    match sys::lock_exclusive_within(&lock, LOCK_WAIT) {
        Ok(true) => {}
        Ok(false) => {
            dir.log("REFUSE busy");
            eprintln!("herdr-link: {name}: another dial is in progress");
            return EXIT_REFUSED;
        }
        Err(err) => {
            dir.log(&format!("REFUSE lock: {err}"));
            return EXIT_REFUSED;
        }
    }
    if let Err(err) = sys::keep_across_exec(lock.as_raw_fd()) {
        dir.log(&format!("REFUSE lock inheritance: {err}"));
        return EXIT_REFUSED;
    }
    dir.log(&format!(
        "LOCKED after {:.3}s",
        started.elapsed().as_secs_f64()
    ));

    let managed_config = if target.ssh_config.is_none() {
        match ssh::write_managed_config(&dir, &account.home) {
            Ok(path) => path,
            Err(err) => {
                dir.log(&format!("REFUSE ssh_config: {err}"));
                return EXIT_REFUSED;
            }
        }
    } else {
        dir.ssh_config_path()
    };
    let args = ssh::ssh_args(&target, &managed_config, line);
    let env = ssh::ssh_env(&account.home, &account.name, &target, std::env::vars_os());
    let _ = dir.write_dial_record(&DialRecord {
        at: now_unix(),
        pid: std::process::id(),
    });

    let (reader, writer) = match std::io::pipe() {
        Ok(pipe) => pipe,
        Err(err) => {
            dir.log(&format!("REFUSE pipe: {err}"));
            return EXIT_REFUSED;
        }
    };
    let ssh_pid = std::process::id();
    match sys::fork() {
        Err(err) => {
            dir.log(&format!("REFUSE fork: {err}"));
            EXIT_REFUSED
        }
        Ok(sys::Forked::Child) => {
            drop(writer);
            drop(lock);
            let reader = OwnedFd::from(reader);
            if sys::detach_watcher(&[reader.as_raw_fd()]).is_ok() {
                dir.log("WATCHER");
                watch(&dir, ssh_pid, &reader, started);
            }
            std::process::exit(0);
        }
        Ok(sys::Forked::Parent) => {
            drop(reader);
            if let Err(err) = sys::stderr_to(OwnedFd::from(writer)) {
                dir.log(&format!("REFUSE stderr: {err}"));
                return EXIT_REFUSED;
            }
            dir.log(&format!("EXEC {}", target.ssh.display()));
            let err = sys::exec_with_env(&target.ssh, "ssh", &args, &env);
            // Only reached when exec failed; the watcher sees EOF and exits.
            dir.log(&format!("EXEC FAILED {err}"));
            drop(lock);
            EXIT_REFUSED
        }
    }
}

/// What a line of ssh's stderr says (D10's classes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StderrEvent {
    /// The far guard found the far core: the link is up.
    Up,
    /// A Tailscale SSH check banner (`needs_check`).
    Check,
    /// The guard found no far core (`far_core_down`).
    FarCoreDown,
    /// Authentication or the host key failed (`auth_failed`).
    AuthFailed,
}

/// Tailscale SSH sends its check as an SSH auth banner, from the control
/// server's message: `# Tailscale SSH requires an additional check.` then
/// `# To authenticate, visit: https://login.tailscale.com/a/<code>`
/// (task A.1; OpenSSH prints auth banners on stderr at LogLevel INFO).
const CHECK_BANNERS: [&str; 2] = [
    "Tailscale SSH requires an additional check",
    "To authenticate, visit:",
];
const AUTH_FAILURES: [&str; 4] = [
    "Permission denied",
    "Host key verification failed",
    "REMOTE HOST IDENTIFICATION HAS CHANGED",
    "host key is known for",
];

pub(crate) fn classify_line(line: &str) -> Option<StderrEvent> {
    if line.trim() == ssh::LINK_UP_MARK {
        Some(StderrEvent::Up)
    } else if CHECK_BANNERS.iter().any(|banner| line.contains(banner)) {
        Some(StderrEvent::Check)
    } else if line.contains(ssh::FAR_CORE_DOWN_TEXT) {
        Some(StderrEvent::FarCoreDown)
    } else if AUTH_FAILURES.iter().any(|text| line.contains(text)) {
        Some(StderrEvent::AuthFailed)
    } else {
        None
    }
}

/// Splits ssh's stderr into lines; a line longer than 4 KiB is cut.
#[derive(Default)]
pub(crate) struct LineSplitter {
    partial: Vec<u8>,
}

const LINE_LIMIT: usize = 4096;

impl LineSplitter {
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        for &byte in bytes {
            if byte == b'\n' {
                lines.push(String::from_utf8_lossy(&self.partial).into_owned());
                self.partial.clear();
            } else if self.partial.len() < LINE_LIMIT {
                self.partial.push(byte);
            }
        }
        lines
    }
}

/// The watcher's decisions over time: given the events and the clock, what
/// to record and whether to end ssh.
#[derive(Debug, Default)]
pub(crate) struct Watch {
    up: bool,
    /// A class was recorded: the dial has failed, and the first class
    /// stands (a banner's second line must not rewrite the gate later).
    closed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Action {
    /// The link is up: a gate left by an expired `far_core_down` goes.
    LinkUp,
    /// Record the class; end ssh when `end` (it would wait otherwise).
    Close { class: GateClass, end: bool },
}

impl Watch {
    pub(crate) fn on_event(&mut self, event: StderrEvent) -> Option<Action> {
        if self.up || self.closed {
            return None;
        }
        let (class, end) = match event {
            StderrEvent::Up => {
                self.up = true;
                return Some(Action::LinkUp);
            }
            StderrEvent::Check => (GateClass::NeedsCheck, true),
            StderrEvent::FarCoreDown => (GateClass::FarCoreDown, false),
            StderrEvent::AuthFailed => (GateClass::AuthFailed, false),
        };
        self.closed = true;
        Some(Action::Close { class, end })
    }

    pub(crate) fn on_tick(&mut self, elapsed: Duration) -> Option<Action> {
        if self.up || self.closed || elapsed < UP_DEADLINE {
            return None;
        }
        self.closed = true;
        Some(Action::Close {
            class: GateClass::AuthStalled,
            end: true,
        })
    }
}

/// The watcher's loop, beside ssh (its parent after the exec).
fn watch(dir: &MachineDir, ssh_pid: u32, reader: &OwnedFd, started: Instant) {
    let mut lines = LineSplitter::default();
    let mut state = Watch::default();
    let mut buf = [0_u8; 4096];
    loop {
        let mut actions = Vec::new();
        // Read before checking the parent: what ssh wrote before it exited
        // (a refusal, the guard's answer) is still in the pipe.
        match sys::read_within(reader.as_raw_fd(), &mut buf, Duration::from_millis(250)) {
            Ok(None) if sys::parent_pid() != ssh_pid => {
                dir.log("SSH GONE");
                return;
            }
            Ok(None) => {}
            Ok(Some(0)) | Err(_) => {
                dir.log("SSH EXITED");
                return;
            }
            Ok(Some(read)) => {
                for line in lines.feed(&buf[..read]) {
                    let event = classify_line(&line);
                    match event {
                        Some(StderrEvent::Check) => dir.log("STDERR tailscale check banner"),
                        Some(StderrEvent::Up) => {}
                        _ => dir.log(&format!("STDERR {:?}", truncate(&line, 160))),
                    }
                    actions.extend(event.and_then(|event| state.on_event(event)));
                }
            }
        }
        actions.extend(state.on_tick(started.elapsed()));
        for action in actions {
            match action {
                Action::LinkUp => {
                    dir.log(&format!("UP after {:.3}s", started.elapsed().as_secs_f64()));
                    let _ = dir.clear_gate();
                }
                Action::Close { class, end } => {
                    let _ = dir.write_gate(&Gate::new(class, None));
                    dir.log(&format!("GATE {}", class.as_str()));
                    if end {
                        end_ssh(dir, ssh_pid);
                    }
                }
            }
        }
    }
}

fn truncate(line: &str, limit: usize) -> String {
    line.chars().take(limit).collect()
}

/// SIGTERM, then SIGKILL after a second, and only while ssh is still this
/// watcher's parent: a pid that changed hands is never signalled.
fn end_ssh(dir: &MachineDir, ssh_pid: u32) {
    if sys::parent_pid() != ssh_pid {
        return;
    }
    dir.log("END ssh");
    sys::signal(ssh_pid, sys::Signal::Term);
    let deadline = Instant::now() + TERM_GRACE;
    while Instant::now() < deadline {
        if sys::parent_pid() != ssh_pid {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if sys::parent_pid() == ssh_pid {
        sys::signal(ssh_pid, sys::Signal::Kill);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = r#"sh -c 'HC_SESSION=x exec "$p" connect'"#;

    fn gate(class: GateClass, at: f64) -> Gate {
        Gate {
            class,
            at,
            detail: None,
        }
    }

    #[test]
    fn admission_follows_line_gate_then_lease() {
        assert_eq!(admit(LINE, None, true, 0.0), Ok(()));
        assert_eq!(
            admit(r#"cmd /d /c "x""#, None, true, 0.0),
            Err(Refusal::NotPosixLine)
        );
        assert_eq!(
            admit(LINE, Some(&gate(GateClass::NeedsCheck, 0.0)), true, 1e9),
            Err(Refusal::Gate(GateClass::NeedsCheck))
        );
        assert_eq!(
            admit(
                LINE,
                Some(&gate(GateClass::FarCoreDown, 100.0)),
                true,
                200.0
            ),
            Err(Refusal::Gate(GateClass::FarCoreDown))
        );
        assert_eq!(
            admit(
                LINE,
                Some(&gate(GateClass::FarCoreDown, 100.0)),
                true,
                400.0
            ),
            Ok(())
        );
        assert_eq!(admit(LINE, None, false, 0.0), Err(Refusal::Dormant));
    }

    #[test]
    fn stderr_lines_classify() {
        assert_eq!(classify_line("HERDR-LINK-UP"), Some(StderrEvent::Up));
        assert_eq!(
            classify_line("# Tailscale SSH requires an additional check."),
            Some(StderrEvent::Check)
        );
        assert_eq!(
            classify_line("# To authenticate, visit: https://login.tailscale.com/a/abc"),
            Some(StderrEvent::Check)
        );
        assert_eq!(
            classify_line(ssh::FAR_CORE_DOWN_TEXT),
            Some(StderrEvent::FarCoreDown)
        );
        assert_eq!(
            classify_line("ac@ac-mbm5: Permission denied (publickey)."),
            Some(StderrEvent::AuthFailed)
        );
        assert_eq!(
            classify_line("Host key verification failed."),
            Some(StderrEvent::AuthFailed)
        );
        assert_eq!(
            classify_line(
                "No ED25519 host key is known for x and you have requested strict checking."
            ),
            Some(StderrEvent::AuthFailed)
        );
        assert_eq!(classify_line("Warning: Permanently added 'x'"), None);
    }

    #[test]
    fn lines_split_across_reads_and_long_lines_are_cut() {
        let mut lines = LineSplitter::default();
        assert!(lines.feed(b"HERDR-LI").is_empty());
        assert_eq!(lines.feed(b"NK-UP\nab"), vec!["HERDR-LINK-UP".to_string()]);
        assert_eq!(lines.feed(b"c\n"), vec!["abc".to_string()]);
        let long = vec![b'x'; LINE_LIMIT * 2];
        lines.feed(&long);
        assert_eq!(lines.feed(b"\n")[0].len(), LINE_LIMIT);
    }

    #[test]
    fn the_deadline_stalls_a_dial_that_is_not_up_once() {
        let mut watch = Watch::default();
        assert_eq!(watch.on_tick(Duration::from_secs(14)), None);
        assert_eq!(
            watch.on_tick(UP_DEADLINE),
            Some(Action::Close {
                class: GateClass::AuthStalled,
                end: true
            })
        );
        assert_eq!(watch.on_tick(Duration::from_secs(16)), None);
    }

    #[test]
    fn an_up_link_has_no_deadline_and_ignores_later_text() {
        let mut watch = Watch::default();
        assert_eq!(watch.on_event(StderrEvent::Up), Some(Action::LinkUp));
        assert_eq!(watch.on_tick(Duration::from_secs(3600)), None);
        assert_eq!(watch.on_event(StderrEvent::AuthFailed), None);
        assert_eq!(watch.on_event(StderrEvent::Up), None);
    }

    #[test]
    fn each_failure_closes_the_gate_and_only_a_check_ends_ssh() {
        for (event, class, end) in [
            (StderrEvent::Check, GateClass::NeedsCheck, true),
            (StderrEvent::FarCoreDown, GateClass::FarCoreDown, false),
            (StderrEvent::AuthFailed, GateClass::AuthFailed, false),
        ] {
            let mut watch = Watch::default();
            assert_eq!(watch.on_event(event), Some(Action::Close { class, end }));
        }
    }

    #[test]
    fn the_first_class_stands() {
        let mut watch = Watch::default();
        assert!(watch.on_event(StderrEvent::Check).is_some());
        assert_eq!(watch.on_event(StderrEvent::Check), None);
        assert_eq!(watch.on_event(StderrEvent::AuthFailed), None);
        assert_eq!(watch.on_tick(UP_DEADLINE), None);
    }

    #[test]
    fn line_shape_shows_kind_and_end() {
        assert_eq!(
            line_shape(LINE),
            format!("{:?}..{:?}", "sh -c '", r#"$p" connect'"#)
        );
    }
}
