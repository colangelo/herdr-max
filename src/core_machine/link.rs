//! `herdr core-link <name> <line>`: the command core runs for every dial of
//! a machine herdr added (core-phase5 D2; prototype `probe/fix/corelink.py`).
//!
//! The order is the design's: the line kind, the gate and leases, the
//! single-flight `dial.lock`, then a watcher is forked and this process
//! **execs ssh**, so the pid core holds (and SIGKILLs after about 19.8 s) is
//! ssh itself and nothing can be orphaned (F2). The watcher holds none of
//! core's streams, only the read end of ssh's stderr; it records the
//! failure classes in `gate.json`, ends a dial that is not up in 15 s, and
//! re-reads the leases every 5 s, ending ssh when none is in force (D3,
//! step 7). Every dial that ends without a link, and every link that drops,
//! is recorded: core gets one retry on its own, then `retry_spent` closes
//! the gate until a person acts (step 9).

use std::fs::OpenOptions;
use std::os::fd::{AsRawFd as _, OwnedFd};
use std::os::unix::fs::OpenOptionsExt as _;
use std::time::{Duration, Instant};

use super::name::MachineName;
use super::ssh;
use super::state::{
    holds_link, newest_act, next_gate, now_unix, DialRecord, Failure, Gate, GateClass, Lease,
    MachineDir,
};
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
/// How often the watcher re-reads the leases (D2 step 7).
pub(crate) const LEASE_RECHECK: Duration = Duration::from_secs(5);
/// How long a link must stay up past `HERDR-LINK-UP` before it counts as
/// up and clears the retry budget (D2 step 9): a far side that answers the
/// guard and then drops at once (a far `connect` that fails, core's own
/// check connection of `machine add`) is a failure, not a link.
pub(crate) const UP_SETTLE: Duration = Duration::from_secs(5);

/// Why a dial was refused before any process or socket was opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Core's Windows fallback line, or any line that is not `sh -c '...'`.
    NotPosixLine,
    /// A recorded failure closes the gate (D10, D2 step 9).
    Gate(GateClass),
    /// No lease in force (D3).
    Dormant,
}

impl Refusal {
    fn describe(&self) -> String {
        match self {
            Self::NotPosixLine => "not core's POSIX line (far sides are macOS and Linux)".into(),
            Self::Gate(GateClass::RetrySpent) => {
                "a dial failed and so did its retry (retry_spent); \
                 input in herdr, herdr machine wake or herdr machine login reopens it"
                    .into()
            }
            Self::Gate(GateClass::FarCoreDown) => "herdr-core is not running there \
                 (far_core_down, 5 min); herdr machine wake reopens it"
                .into(),
            Self::Gate(class) => format!(
                "the gate is closed ({}); run herdr machine login",
                class.as_str()
            ),
            Self::Dormant => "no lease in force".into(),
        }
    }
}

/// D2 steps 1 and 2, decided from the files alone: the line kind, then the
/// gate (whose `retry_spent` yields only to a lease from a newer act), then
/// a lease in force.
pub(crate) fn admit(
    line: &str,
    gate: Option<&Gate>,
    leases: &[Lease],
    now: f64,
) -> Result<(), Refusal> {
    if !ssh::is_posix_line(line) {
        return Err(Refusal::NotPosixLine);
    }
    let act = newest_act(leases, now);
    if let Some(gate) = gate.filter(|gate| gate.closes_at(now, act)) {
        return Err(Refusal::Gate(gate.class));
    }
    if act.is_none() {
        return Err(Refusal::Dormant);
    }
    Ok(())
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
    if let Err(refusal) = admit(line, gate.as_ref(), &dir.leases(), now_unix()) {
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

    let config = match ssh::write_link_config(&dir, &target, &account.home) {
        Ok(path) => path,
        Err(err) => {
            dir.log(&format!("REFUSE ssh_config: {err}"));
            return EXIT_REFUSED;
        }
    };
    let args = ssh::ssh_args(&target, &config, line);
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
    /// When `HERDR-LINK-UP` came, from the start.
    up_at: Option<Duration>,
    /// Up for [`UP_SETTLE`]: the budget is cleared, a drop is a first
    /// failure.
    settled: bool,
    /// A failure was recorded: the first one stands (a banner's second line
    /// must not rewrite the gate later, nor ssh's exit after it).
    closed: bool,
    /// The watcher ended ssh because no lease is in force: not a failure.
    lease_ended: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Action {
    /// The far guard answered.
    LinkUp,
    /// The link stayed up: the gate and the retry budget are cleared.
    Settled,
    /// Record the failure; end ssh when `end` (it would wait otherwise).
    Close { failure: Failure, end: bool },
    /// No lease in force: end ssh, record nothing (D2 step 7).
    LeaseEnded,
}

impl Watch {
    pub(crate) fn on_event(&mut self, event: StderrEvent, elapsed: Duration) -> Option<Action> {
        if self.up_at.is_some() || self.closed || self.lease_ended {
            return None;
        }
        let (class, end) = match event {
            StderrEvent::Up => {
                self.up_at = Some(elapsed);
                return Some(Action::LinkUp);
            }
            StderrEvent::Check => (GateClass::NeedsCheck, true),
            StderrEvent::FarCoreDown => (GateClass::FarCoreDown, false),
            StderrEvent::AuthFailed => (GateClass::AuthFailed, false),
        };
        self.closed = true;
        Some(Action::Close {
            failure: Failure::Class(class),
            end,
        })
    }

    pub(crate) fn on_tick(&mut self, elapsed: Duration) -> Option<Action> {
        if self.closed || self.lease_ended || self.settled {
            return None;
        }
        match self.up_at {
            Some(up_at) if elapsed.saturating_sub(up_at) >= UP_SETTLE => {
                self.settled = true;
                Some(Action::Settled)
            }
            Some(_) => None,
            None if elapsed >= UP_DEADLINE => {
                self.closed = true;
                Some(Action::Close {
                    failure: Failure::Class(GateClass::AuthStalled),
                    end: true,
                })
            }
            None => None,
        }
    }

    /// The lease re-read found none in force.
    pub(crate) fn on_no_lease(&mut self) -> Option<Action> {
        if self.lease_ended || self.closed {
            return None;
        }
        self.lease_ended = true;
        Some(Action::LeaseEnded)
    }

    /// ssh is gone: a dial that never came up, or a link that dropped, is a
    /// failure, unless one stands already or the lease ended it.
    pub(crate) fn on_ssh_end(&mut self) -> Option<Action> {
        if self.closed || self.lease_ended {
            return None;
        }
        self.closed = true;
        let why = match (self.up_at, self.settled) {
            (_, true) => "the link dropped",
            (Some(_), false) => "the link dropped as soon as it came up",
            (None, _) => "ssh ended before the far side answered",
        };
        Some(Action::Close {
            failure: Failure::Ended(why),
            end: false,
        })
    }
}

/// Whether a lease is in force now, one that only keeps the link included
/// (a fleet CLI call's, D7).
fn leased(dir: &MachineDir) -> bool {
    holds_link(&dir.leases(), now_unix())
}

/// The watcher's loop, beside ssh (its parent after the exec).
fn watch(dir: &MachineDir, ssh_pid: u32, reader: &OwnedFd, started: Instant) {
    let mut lines = LineSplitter::default();
    let mut state = Watch::default();
    let mut buf = [0_u8; 4096];
    let mut next_lease_check = started + LEASE_RECHECK;
    loop {
        let mut actions = Vec::new();
        let mut gone = None;
        // Read before checking the parent: what ssh wrote before it exited
        // (a refusal, the guard's answer) is still in the pipe.
        // Wake for the next lease re-read on time, so a lease end is seen
        // within 5 s, not 5 s plus a read's wait each round.
        let wait = next_lease_check
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(250));
        match sys::read_within(reader.as_raw_fd(), &mut buf, wait) {
            Ok(None) if sys::parent_pid() != ssh_pid => gone = Some("SSH GONE"),
            Ok(None) => {}
            Ok(Some(0)) | Err(_) => gone = Some("SSH EXITED"),
            Ok(Some(read)) => {
                for line in lines.feed(&buf[..read]) {
                    let event = classify_line(&line);
                    match event {
                        Some(StderrEvent::Check) => dir.log("STDERR tailscale check banner"),
                        Some(StderrEvent::Up) => {}
                        _ => dir.log(&format!("STDERR {:?}", truncate(&line, 160))),
                    }
                    actions
                        .extend(event.and_then(|event| state.on_event(event, started.elapsed())));
                }
            }
        }
        if let Some(how) = gone {
            dir.log(how);
            actions.extend(state.on_ssh_end());
        } else {
            actions.extend(state.on_tick(started.elapsed()));
            if Instant::now() >= next_lease_check {
                next_lease_check += LEASE_RECHECK;
                if !leased(dir) {
                    actions.extend(state.on_no_lease());
                }
            }
        }
        for action in actions {
            apply(dir, ssh_pid, started, action);
        }
        if gone.is_some() {
            return;
        }
    }
}

fn apply(dir: &MachineDir, ssh_pid: u32, started: Instant, action: Action) {
    let elapsed = started.elapsed().as_secs_f64();
    match action {
        Action::LinkUp => dir.log(&format!("UP after {elapsed:.3}s")),
        Action::Settled => {
            dir.log(&format!("SETTLED after {elapsed:.3}s"));
            let _ = dir.clear_gate();
        }
        Action::Close { failure, end } => {
            // The gate as recorded now, not at admission: a failure another
            // dial wrote meanwhile counts too.
            let current = dir.read_gate().ok().flatten();
            let gate = next_gate(current.as_ref(), failure, now_unix());
            let _ = dir.write_gate(&gate);
            match &gate.detail {
                Some(detail) => dir.log(&format!("GATE {} ({detail})", gate.class.as_str())),
                None => dir.log(&format!("GATE {}", gate.class.as_str())),
            }
            if end {
                end_ssh(dir, ssh_pid);
            }
        }
        Action::LeaseEnded => {
            dir.log("LEASE END: no lease in force");
            end_ssh(dir, ssh_pid);
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

    fn lease(expires_at: f64, input_at: f64) -> Lease {
        Lease {
            holder: crate::core_machine::state::LeaseHolder::manual(),
            expires_at,
            input_at,
            dials: true,
        }
    }

    #[test]
    fn admission_follows_line_gate_then_lease() {
        let leased = [lease(1e10, 0.0)];
        assert_eq!(admit(LINE, None, &leased, 0.0), Ok(()));
        assert_eq!(
            admit(r#"cmd /d /c "x""#, None, &leased, 0.0),
            Err(Refusal::NotPosixLine)
        );
        assert_eq!(
            admit(LINE, Some(&gate(GateClass::NeedsCheck, 0.0)), &leased, 1e9),
            Err(Refusal::Gate(GateClass::NeedsCheck))
        );
        assert_eq!(
            admit(
                LINE,
                Some(&gate(GateClass::FarCoreDown, 100.0)),
                &leased,
                200.0
            ),
            Err(Refusal::Gate(GateClass::FarCoreDown))
        );
        assert_eq!(
            admit(
                LINE,
                Some(&gate(GateClass::FarCoreDown, 100.0)),
                &leased,
                400.0
            ),
            Ok(())
        );
    }

    /// D7 (2.2): a fleet CLI's keep lease on a connected machine admits no
    /// dial, so core's retry after a drop during the call is refused.
    #[test]
    fn a_keep_lease_alone_admits_no_dial() {
        let keep = [Lease {
            dials: false,
            ..lease(1e10, 0.0)
        }];
        assert_eq!(admit(LINE, None, &keep, 100.0), Err(Refusal::Dormant));
        assert_eq!(
            admit(LINE, Some(&gate(GateClass::DialFailed, 90.0)), &keep, 100.0),
            Err(Refusal::Dormant)
        );
        let both = [keep[0].clone(), lease(1e10, 50.0)];
        assert_eq!(admit(LINE, None, &both, 100.0), Ok(()));
    }

    /// D7 (2.2): the watcher keeps an up link while a keep lease holds,
    /// so a fleet CLI call is not cut when every other lease ends.
    #[test]
    fn a_keep_lease_keeps_the_link_up() {
        let root = crate::core_machine::state::tests::scratch("keep-link");
        let dir = MachineDir::at(&root, &MachineName::parse("loop").expect("name"));
        dir.create().expect("create");
        assert!(!leased(&dir));
        dir.write_keep_lease(
            // A running holder: a gone one's lease is not in force (H7).
            &crate::core_machine::state::LeaseHolder::cli(std::process::id()),
            now_unix() + 60.0,
        )
        .expect("keep");
        assert!(leased(&dir));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// V1's unit half: no lease, or only expired ones, is no dial.
    #[test]
    fn no_lease_in_force_is_dormant() {
        assert_eq!(admit(LINE, None, &[], 0.0), Err(Refusal::Dormant));
        assert_eq!(
            admit(LINE, None, &[lease(99.0, 0.0), lease(100.0, 50.0)], 100.0),
            Err(Refusal::Dormant)
        );
        assert_eq!(admit(LINE, None, &[lease(100.1, 0.0)], 100.0), Ok(()));
        // The Windows line is refused before anything else, lease or not.
        assert_eq!(
            admit("cmd /d /c x", None, &[], 0.0),
            Err(Refusal::NotPosixLine)
        );
    }

    /// D2 step 9 at admission: one failure lets the retry through;
    /// `retry_spent` yields only to a lease from an act after it, never to
    /// a lease kept alive with no new act.
    #[test]
    fn retry_spent_admits_only_a_newer_act() {
        let failed = gate(GateClass::DialFailed, 100.0);
        assert_eq!(
            admit(LINE, Some(&failed), &[lease(1e10, 50.0)], 101.0),
            Ok(())
        );
        assert_eq!(
            admit(LINE, Some(&failed), &[], 101.0),
            Err(Refusal::Dormant)
        );
        let spent = gate(GateClass::RetrySpent, 100.0);
        assert_eq!(
            admit(LINE, Some(&spent), &[lease(1e10, 50.0)], 500.0),
            Err(Refusal::Gate(GateClass::RetrySpent))
        );
        assert_eq!(
            admit(
                LINE,
                Some(&spent),
                &[lease(1e10, 50.0), lease(1e10, 150.0)],
                500.0
            ),
            Ok(())
        );
        // A newer act on an expired lease does not count.
        assert_eq!(
            admit(
                LINE,
                Some(&spent),
                &[lease(1e10, 50.0), lease(400.0, 150.0)],
                500.0
            ),
            Err(Refusal::Gate(GateClass::RetrySpent))
        );
        // The login classes yield to no lease.
        assert_eq!(
            admit(
                LINE,
                Some(&gate(GateClass::AuthFailed, 100.0)),
                &[lease(1e10, 150.0)],
                500.0
            ),
            Err(Refusal::Gate(GateClass::AuthFailed))
        );
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

    fn close(class: GateClass, end: bool) -> Option<Action> {
        Some(Action::Close {
            failure: Failure::Class(class),
            end,
        })
    }

    #[test]
    fn the_deadline_stalls_a_dial_that_is_not_up_once() {
        let mut watch = Watch::default();
        assert_eq!(watch.on_tick(Duration::from_secs(14)), None);
        assert_eq!(
            watch.on_tick(UP_DEADLINE),
            close(GateClass::AuthStalled, true)
        );
        assert_eq!(watch.on_tick(Duration::from_secs(16)), None);
        assert_eq!(
            watch.on_ssh_end(),
            None,
            "the stall stands, not a second failure"
        );
    }

    #[test]
    fn an_up_link_settles_and_ignores_later_text() {
        let mut watch = Watch::default();
        let at = Duration::from_secs(1);
        assert_eq!(watch.on_event(StderrEvent::Up, at), Some(Action::LinkUp));
        assert_eq!(
            watch.on_tick(at + UP_SETTLE - Duration::from_millis(1)),
            None
        );
        assert_eq!(watch.on_tick(at + UP_SETTLE), Some(Action::Settled));
        assert_eq!(watch.on_tick(Duration::from_secs(3600)), None);
        assert_eq!(watch.on_event(StderrEvent::AuthFailed, at), None);
        assert_eq!(watch.on_event(StderrEvent::Up, at), None);
    }

    #[test]
    fn each_failure_closes_the_gate_and_only_a_check_ends_ssh() {
        for (event, class, end) in [
            (StderrEvent::Check, GateClass::NeedsCheck, true),
            (StderrEvent::FarCoreDown, GateClass::FarCoreDown, false),
            (StderrEvent::AuthFailed, GateClass::AuthFailed, false),
        ] {
            let mut watch = Watch::default();
            assert_eq!(watch.on_event(event, Duration::ZERO), close(class, end));
            assert_eq!(watch.on_ssh_end(), None, "{class:?} stands at ssh's exit");
        }
    }

    #[test]
    fn the_first_class_stands() {
        let mut watch = Watch::default();
        assert!(watch.on_event(StderrEvent::Check, Duration::ZERO).is_some());
        assert_eq!(watch.on_event(StderrEvent::Check, Duration::ZERO), None);
        assert_eq!(
            watch.on_event(StderrEvent::AuthFailed, Duration::ZERO),
            None
        );
        assert_eq!(watch.on_tick(UP_DEADLINE), None);
    }

    /// D2 step 9: every way a dial ends without a lasting link is recorded.
    #[test]
    fn a_dial_or_link_that_ends_is_a_failure() {
        let ended = |why| {
            Some(Action::Close {
                failure: Failure::Ended(why),
                end: false,
            })
        };
        let mut never_up = Watch::default();
        assert_eq!(
            never_up.on_ssh_end(),
            ended("ssh ended before the far side answered")
        );
        assert_eq!(never_up.on_ssh_end(), None);

        let mut flapped = Watch::default();
        flapped.on_event(StderrEvent::Up, Duration::ZERO);
        assert_eq!(
            flapped.on_ssh_end(),
            ended("the link dropped as soon as it came up")
        );

        let mut dropped = Watch::default();
        dropped.on_event(StderrEvent::Up, Duration::ZERO);
        dropped.on_tick(UP_SETTLE);
        assert_eq!(dropped.on_ssh_end(), ended("the link dropped"));
    }

    /// D2 step 7: the lease end ends ssh once and is no failure, before or
    /// after the link came up.
    #[test]
    fn the_lease_end_ends_ssh_and_records_nothing() {
        let mut up = Watch::default();
        up.on_event(StderrEvent::Up, Duration::ZERO);
        up.on_tick(UP_SETTLE);
        assert_eq!(up.on_no_lease(), Some(Action::LeaseEnded));
        assert_eq!(up.on_no_lease(), None);
        assert_eq!(up.on_ssh_end(), None);

        let mut dialling = Watch::default();
        assert_eq!(dialling.on_no_lease(), Some(Action::LeaseEnded));
        assert_eq!(
            dialling.on_tick(UP_DEADLINE),
            None,
            "no stall after the lease ended it"
        );
        assert_eq!(dialling.on_ssh_end(), None);

        let mut stalled = Watch::default();
        stalled.on_tick(UP_DEADLINE);
        assert_eq!(stalled.on_no_lease(), None, "the stall already ends ssh");
    }

    #[test]
    fn line_shape_shows_kind_and_end() {
        assert_eq!(
            line_shape(LINE),
            format!("{:?}..{:?}", "sh -c '", r#"$p" connect'"#)
        );
    }
}
