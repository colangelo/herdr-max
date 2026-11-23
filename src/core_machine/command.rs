//! `herdr machine add --core`, `rm --core`, `login` and `status --core`
//! (core-phase5 D1, D9, D10). Adding and logging in are a person's acts in
//! a terminal; status is for agents to read before they act.

use std::io::IsTerminal as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use super::dialer::{self, Outcome, PrimaryCore};
use super::name::MachineName;
use super::ssh;
use super::state::{
    is_ssh_destination, newest_act, now_unix, passwd_machines_root, Gate, GateClass, LeaseHolder,
    MachineDir, Target, MANUAL_LEASE_SECS, MAX_WAKE_SECS,
};
use crate::pane::core_machines as core;

const HELP: &str = "  herdr machine add --core <ssh-target> [--name <name>] [--remote-session <core-session>] [--ssh-config <file>]
  herdr machine rm --core <name>
  herdr machine login <name>
  herdr machine wake <name> [--for <30m|2h|90s>]
  herdr machine status --core [<name>] [--json]";

pub(crate) fn add(args: &[String]) -> std::io::Result<i32> {
    let parsed = match parse_add(args) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("{err}");
            return Ok(2);
        }
    };
    if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
        eprintln!(
            "herdr machine add --core needs a terminal: hc may ask before it copies itself there, and ssh may prompt.\nA person runs it: {}",
            parsed.command_line()
        );
        return Ok(2);
    }
    let Some(ssh_program) = find_on_path("ssh") else {
        eprintln!("no ssh on PATH");
        return Ok(1);
    };
    let herdr = crate::platform::launch_executable()?;
    if !herdr.is_absolute() {
        eprintln!("cannot tell the absolute path of this herdr");
        return Ok(1);
    }
    let dir = MachineDir::at(&passwd_machines_root()?, &parsed.name);
    if dir.target_path().exists() {
        eprintln!(
            "{} is already a core machine here; remove it first: herdr machine rm --core {}",
            parsed.name, parsed.name
        );
        return Ok(1);
    }
    let created = !dir.exists();
    dir.create()?;
    let target = Target {
        target: parsed.target.clone(),
        session: parsed.session.clone(),
        ssh: ssh_program,
        ssh_config: parsed.ssh_config.clone(),
        ssh_auth_sock: std::env::var_os("SSH_AUTH_SOCK")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute()),
    };
    target.validate()?;
    dir.write_target(&target)?;
    dir.clear_gate()?;
    // D3, F8: the lease comes before hc dials (twice, P1).
    let now = now_unix();
    dir.write_lease(&LeaseHolder::manual(), now + MANUAL_LEASE_SECS, now)?;

    let via = format!(
        "{} core-link {}",
        sh_quote(&herdr.to_string_lossy()),
        parsed.name
    );
    let mut hc = match core::hc_command() {
        Ok(hc) => hc,
        Err(err) => {
            undo_add(&dir, created);
            eprintln!("{err}");
            return Ok(1);
        }
    };
    // Never --install (D9): hc asks in this terminal when it would copy itself.
    hc.args(["machine", "add", "--name", parsed.name.as_str()])
        .args(["--session", &parsed.session])
        .args(["--via", &via]);
    let status = hc.status()?;
    if !status.success() {
        if let Ok(log) = std::fs::read_to_string(dir.log_path()) {
            let lines: Vec<&str> = log.lines().collect();
            eprintln!("herdr core-link's log for these dials:");
            for line in &lines[lines.len().saturating_sub(8)..] {
                eprintln!("  {line}");
            }
        }
        undo_add(&dir, created);
        eprintln!("hc machine add failed; {} was not added", parsed.name);
        return Ok(status.code().unwrap_or(1));
    }
    println!(
        "Added core machine {name} to herdr-core session {session}. Its link runs `herdr core-link {name}`: \
         every dial is a fresh ssh login, and a failed one closes the gate until `herdr machine login {name}`.",
        name = parsed.name,
        session = core::primary_session(),
    );
    Ok(0)
}

fn undo_add(dir: &MachineDir, created: bool) {
    let result = if created {
        std::fs::remove_dir_all(dir.path())
    } else {
        std::fs::remove_file(dir.target_path())
    };
    if let Err(err) = result {
        tracing::warn!(%err, "could not remove the state of a machine that was not added");
    }
}

#[derive(Debug, PartialEq, Eq)]
struct AddArgs {
    target: String,
    name: MachineName,
    session: String,
    ssh_config: Option<PathBuf>,
}

impl AddArgs {
    fn command_line(&self) -> String {
        let mut line = format!(
            "herdr machine add --core {} --name {} --remote-session {}",
            self.target, self.name, self.session
        );
        if let Some(config) = &self.ssh_config {
            line.push_str(&format!(
                " --ssh-config {}",
                sh_quote(&config.to_string_lossy())
            ));
        }
        line
    }
}

fn parse_add(args: &[String]) -> Result<AddArgs, String> {
    let usage = || format!("usage:\n{HELP}");
    let mut target = None;
    let mut name = None;
    let mut session = None;
    let mut ssh_config = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let slot = match arg.as_str() {
            "--core" => continue,
            "--name" => &mut name,
            "--remote-session" => &mut session,
            "--ssh-config" => &mut ssh_config,
            positional if !positional.starts_with('-') && target.is_none() => {
                target = Some(positional.to_string());
                continue;
            }
            other => {
                return Err(format!(
                    "unknown machine add --core option: {other}\n{}",
                    usage()
                ))
            }
        };
        let value = rest
            .next()
            .ok_or_else(|| format!("missing value for {arg}"))?;
        if slot.replace(value.clone()).is_some() {
            return Err(format!("{arg} can only be given once"));
        }
    }
    let target = target.ok_or_else(usage)?;
    if !is_ssh_destination(&target) {
        return Err(format!("{target:?} is not an ssh destination herdr passes"));
    }
    let name = MachineName::parse(&name.unwrap_or_else(|| default_name(&target)))
        .map_err(|err| format!("{err}; pass --name"))?;
    let session = session.unwrap_or_else(|| "default".into());
    if !super::name::is_plain_name(&session) {
        return Err(format!("core session {session:?} is not a plain name"));
    }
    let ssh_config = ssh_config.map(PathBuf::from);
    if let Some(config) = &ssh_config {
        if !config.is_absolute() || !config.is_file() {
            return Err(format!(
                "--ssh-config {} is not an absolute path to a file",
                config.display()
            ));
        }
    }
    Ok(AddArgs {
        target,
        name,
        session,
        ssh_config,
    })
}

/// The host of an ssh destination: `ac@ac-mbm5` → `ac-mbm5`,
/// `ssh://u@[::1]:22` → `::1` (refused later as a name).
fn default_name(target: &str) -> String {
    let authority = target.strip_prefix("ssh://").unwrap_or(target);
    let authority = authority.split('/').next().unwrap_or(authority);
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    match host.strip_prefix('[') {
        Some(bracketed) => bracketed.split(']').next().unwrap_or(bracketed).to_string(),
        None => host.split(':').next().unwrap_or(host).to_string(),
    }
}

/// One shell word for an sh line (core runs the via as one).
fn sh_quote(word: &str) -> String {
    if !word.is_empty()
        && word
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/._-+".contains(&byte))
    {
        return word.to_string();
    }
    format!("'{}'", word.replace('\'', r"'\''"))
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt as _;
    std::env::split_paths(&std::env::var_os("PATH")?)
        .filter(|folder| folder.is_absolute())
        .map(|folder| folder.join(name))
        .find(|candidate| {
            std::fs::metadata(candidate)
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
}

fn machine_arg(args: &[String], usage: &str) -> Result<MachineName, String> {
    let names: Vec<&String> = args.iter().filter(|arg| *arg != "--core").collect();
    match names.as_slice() {
        [name] => MachineName::parse(name).map_err(|err| err.to_string()),
        _ => Err(format!("usage: {usage}")),
    }
}

pub(crate) fn remove(args: &[String]) -> std::io::Result<i32> {
    let name = match machine_arg(args, "herdr machine rm --core <name>") {
        Ok(name) => name,
        Err(err) => {
            eprintln!("{err}");
            return Ok(2);
        }
    };
    let mut hc = match core::hc_command() {
        Ok(hc) => hc,
        Err(err) => {
            eprintln!("{err}");
            return Ok(1);
        }
    };
    let status = hc.args(["machine", "rm", name.as_str()]).status()?;
    let dir = MachineDir::at(&passwd_machines_root()?, &name);
    if dir.exists() {
        remove_link_state(&dir)?;
        println!("Removed herdr's link state for {name}.");
    }
    Ok(status.code().unwrap_or(1))
}

/// Removes a machine's folder. The link core just ended may still be
/// winding down for a moment: its watcher writes the end to `link.log` and
/// the gate, which can land between the removal's listing and its last
/// `rmdir`. So a removal that fails is tried again for up to 2 s.
fn remove_link_state(dir: &MachineDir) -> std::io::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match std::fs::remove_dir_all(dir.path()) {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) if std::time::Instant::now() >= deadline => return Err(err),
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    }
}

pub(crate) fn login(args: &[String]) -> std::io::Result<i32> {
    let name = match machine_arg(args, "herdr machine login <name>") {
        Ok(name) => name,
        Err(err) => {
            eprintln!("{err}");
            return Ok(2);
        }
    };
    if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
        eprintln!(
            "herdr machine login needs a terminal: ssh may ask for a key, a host key or a Tailscale check.\nA person runs it: herdr machine login {name}"
        );
        return Ok(2);
    }
    let account = crate::platform::core_link::passwd_account()?;
    let dir = MachineDir::at(&super::state::machines_root(&account.home), &name);
    let target = match dir.check().and_then(|()| dir.read_target()) {
        Ok(target) => target,
        Err(err) => {
            eprintln!("{name} is not a herdr core machine here: {err}");
            return Ok(1);
        }
    };
    // D3, F8: the lease comes before the connect it leads to.
    let now = now_unix();
    dir.write_lease(&LeaseHolder::manual(), now + MANUAL_LEASE_SECS, now)?;
    let config = ssh::write_link_config(&dir, &target, &account.home)?;
    let status = std::process::Command::new(&target.ssh)
        .arg0("ssh")
        .args(ssh::login_ssh_args(&target, &config))
        .env_clear()
        .envs(ssh::ssh_env(
            &account.home,
            &account.name,
            &target,
            std::env::vars_os(),
        ))
        .status()?;
    if !status.success() {
        eprintln!("ssh to {} failed; the gate is unchanged", target.target);
        return Ok(1);
    }
    dir.clear_gate()?;
    dir.log("LOGIN cleared the gate");
    Ok(report_wake(&name, &dir))
}

/// `herdr machine wake <name> [--for D]` (D3, S4b): a `manual` lease for
/// the given time (30 min by default, at most 12 h), then one connect
/// through the dialer. It reopens `far_core_down` at once and, by its newer
/// lease, `retry_spent`; the login classes stay for `herdr machine login`.
/// A wake is an announced dial: an agent says so on the desk first.
pub(crate) fn wake(args: &[String]) -> std::io::Result<i32> {
    let (name, secs) = match parse_wake(args) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("{err}");
            return Ok(2);
        }
    };
    let dir = MachineDir::at(&passwd_machines_root()?, &name);
    if let Err(err) = dir.check().and_then(|()| dir.read_target().map(|_| ())) {
        eprintln!("{name} is not a herdr core machine here: {err}");
        return Ok(1);
    }
    let gate = match dir.read_gate() {
        Ok(gate) => gate,
        Err(err) => {
            eprintln!("{name}: unreadable gate.json ({err}); run herdr machine login {name}");
            return Ok(1);
        }
    };
    if let Some(gate) = gate.as_ref().filter(|gate| {
        matches!(
            gate.class,
            GateClass::NeedsCheck | GateClass::AuthStalled | GateClass::AuthFailed
        )
    }) {
        eprintln!(
            "{name}: the gate is closed ({}); a wake cannot reopen it. A person runs: herdr machine login {name}",
            gate.class.as_str()
        );
        return Ok(1);
    }
    let now = now_unix();
    // D3: the lease before the connect it leads to.
    dir.write_lease(&LeaseHolder::manual(), now + secs, now)?;
    if gate.is_some_and(|gate| gate.class == GateClass::FarCoreDown) {
        dir.clear_gate()?;
        dir.log("WAKE cleared far_core_down");
    }
    dir.log(&format!("WAKE lease for {secs:.0}s"));
    Ok(report_wake(&name, &dir))
}

/// One connect through the dialer's rules (D3a), and its result printed.
fn report_wake(name: &MachineName, dir: &MachineDir) -> i32 {
    let outcome = dialer::dial_once(
        &PrimaryCore,
        name,
        &dir.connect_lock_path(),
        dialer::Timing::default(),
    );
    match &outcome {
        Outcome::NotListed => {
            println!("{name}: herdr-core does not list it (or is not running), so nothing dialled");
            0
        }
        Outcome::Failed(err) => {
            eprintln!("{name}: machine.connect failed: {err}");
            1
        }
        _ => {
            let state = outcome.state().unwrap_or("unknown");
            println!("{name}: {state}");
            i32::from(state != "connected")
        }
    }
}

fn parse_wake(args: &[String]) -> Result<(MachineName, f64), String> {
    let usage = "usage: herdr machine wake <name> [--for <30m|2h|90s>]";
    let mut name = None;
    let mut secs = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--core" => {}
            "--for" if secs.is_none() => {
                let raw = rest.next().ok_or(usage)?;
                secs = Some(parse_duration(raw)?);
            }
            raw if !raw.starts_with('-') && name.is_none() => {
                name = Some(MachineName::parse(raw).map_err(|err| err.to_string())?);
            }
            _ => return Err(usage.into()),
        }
    }
    let name = name.ok_or(usage)?;
    let secs = secs.unwrap_or(MANUAL_LEASE_SECS);
    if secs > MAX_WAKE_SECS {
        return Err(format!("--for is at most 12h ({} s)", MAX_WAKE_SECS as u64));
    }
    Ok((name, secs))
}

/// `90`, `90s`, `30m` or `2h`, more than zero.
fn parse_duration(raw: &str) -> Result<f64, String> {
    let bad = || format!("{raw:?} is not a duration like 90s, 30m or 2h");
    let (number, unit) = match raw.find(|c: char| !c.is_ascii_digit()) {
        Some(at) => raw.split_at(at),
        None => (raw, "s"),
    };
    let number: u64 = number.parse().map_err(|_| bad())?;
    let scale = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        _ => return Err(bad()),
    };
    let secs = number.checked_mul(scale).ok_or_else(bad)?;
    if secs == 0 {
        return Err(bad());
    }
    Ok(secs as f64)
}

#[derive(Debug, Serialize)]
struct LeaseRow {
    holder: String,
    expires_at: f64,
    /// The human act that wrote it; only an act newer than a
    /// `retry_spent` failure reopens the gate (D2 step 9).
    input_at: f64,
    in_force: bool,
    /// False for a fleet CLI call's lease on a connected machine: it keeps
    /// the link up and admits no dial (D7).
    dials: bool,
}

#[derive(Debug, Serialize)]
struct FarHerdrRow {
    path: String,
    version: Option<String>,
}

#[derive(Debug, Serialize)]
struct StatusRow {
    name: String,
    /// Core's state for it (`connected`, `connecting`, `unreachable`, ...),
    /// `absent` when core does not list it, `unknown` when core is down.
    state: String,
    /// `open`, `dormant` (no lease in force) or the class that closes it;
    /// `null` for a machine herdr did not add.
    gate: Option<String>,
    /// The class recorded in `gate.json`, closing or not (`dial_failed`
    /// closes nothing; an expired `far_core_down` neither).
    gate_record: Option<String>,
    gate_at: Option<f64>,
    leases: Vec<LeaseRow>,
    last_dial: Option<f64>,
    /// Whether core reaches it through herdr's link command.
    herdr_link: bool,
    /// The far herdr `--machine` found over the current link (D7), when
    /// one did.
    far_herdr: Option<FarHerdrRow>,
    target: Option<String>,
    session: Option<String>,
}

pub(crate) fn status(args: &[String]) -> std::io::Result<i32> {
    let mut json_out = false;
    let mut only = None;
    for arg in args {
        match arg.as_str() {
            "--core" => {}
            "--json" if !json_out => json_out = true,
            name if !name.starts_with('-') && only.is_none() => match MachineName::parse(name) {
                Ok(name) => only = Some(name),
                Err(err) => {
                    eprintln!("{err}");
                    return Ok(2);
                }
            },
            _ => {
                eprintln!("usage: herdr machine status --core [<name>] [--json]");
                return Ok(2);
            }
        }
    }
    let root = passwd_machines_root()?;
    let listed = match core::call_running_primary("machine.list", json!({}), Duration::from_secs(5))
    {
        Ok(Some(reply)) => Some(reply),
        Ok(None) => None,
        Err(err) => {
            eprintln!("herdr-core did not list its machines: {err}");
            None
        }
    };
    let mut rows = status_rows(&root, listed.as_ref(), now_unix());
    if let Some(only) = &only {
        rows.retain(|row| row.name == only.as_str());
        if rows.is_empty() {
            eprintln!("no core machine {only}");
            return Ok(1);
        }
    }
    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&rows).map_err(std::io::Error::other)?
        );
        return Ok(0);
    }
    if rows.is_empty() {
        println!("No core machines.");
    }
    for row in &rows {
        let leases: Vec<String> = row
            .leases
            .iter()
            .filter(|lease| lease.in_force)
            .map(|lease| {
                if lease.dials {
                    lease.holder.clone()
                } else {
                    format!("{}(keep)", lease.holder)
                }
            })
            .collect();
        println!(
            "{}\t{}\tgate {}\tleases {}",
            row.name,
            row.state,
            row.gate.as_deref().unwrap_or("-"),
            if leases.is_empty() {
                "-".to_string()
            } else {
                leases.join(",")
            }
        );
    }
    Ok(0)
}

/// A core machine herdr added and links (its state folder holds
/// `target.json`), with its gate as `machine status` reports it (`open`,
/// `dormant`, or a closed class), read from files only: no core call, no
/// dial (core-phase5 D6, C.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkedMachine {
    pub name: MachineName,
    pub gate: String,
    /// The ssh destination it was added with (`ac@ac-mbm5`).
    pub target: Option<String>,
}

/// The host of an ssh destination (`ac@ac-mbm5` → `ac-mbm5`), lowercased.
fn destination_host(destination: &str) -> String {
    destination
        .rsplit('@')
        .next()
        .unwrap_or(destination)
        .to_ascii_lowercase()
}

/// Saved herdr machines and core machines that reach the same host, as
/// (saved label, core machine) pairs (D8: both are shown, with one notice).
/// The endpoint handshake carries no far core id, so the hosts of their ssh
/// destinations are compared.
pub(crate) fn same_host_pairs(
    saved: &[(String, String)],
    core: &[LinkedMachine],
) -> Vec<(String, MachineName)> {
    let mut pairs = Vec::new();
    for machine in core {
        let Some(target) = machine.target.as_deref() else {
            continue;
        };
        let host = destination_host(target);
        for (label, destination) in saved {
            if destination_host(destination) == host {
                pairs.push((label.clone(), machine.name.clone()));
            }
        }
    }
    pairs
}

/// The core machines herdr links, by name; empty when the state root
/// cannot be read.
pub(crate) fn linked_machines() -> Vec<LinkedMachine> {
    let Ok(root) = passwd_machines_root() else {
        return Vec::new();
    };
    linked_machines_in(&root, now_unix())
}

fn linked_machines_in(root: &Path, now: f64) -> Vec<LinkedMachine> {
    status_rows(root, None, now)
        .into_iter()
        .filter(|row| row.target.is_some())
        .filter_map(|row| {
            Some(LinkedMachine {
                name: MachineName::parse(&row.name).ok()?,
                gate: row.gate.unwrap_or_else(|| "open".into()),
                target: row.target,
            })
        })
        .collect()
}

fn status_rows(root: &Path, listed: Option<&Value>, now: f64) -> Vec<StatusRow> {
    let mut names: Vec<MachineName> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| MachineName::parse(entry.file_name().to_str()?).ok())
        .collect();
    let core_machines: Vec<&Value> = listed
        .and_then(|reply| reply["machines"].as_array())
        .into_iter()
        .flatten()
        .collect();
    names.extend(
        core_machines
            .iter()
            .filter_map(|machine| MachineName::parse(machine["name"].as_str()?).ok()),
    );
    names.sort();
    names.dedup();
    names
        .into_iter()
        .map(|name| {
            let core_record = core_machines
                .iter()
                .find(|machine| machine["name"] == name.as_str());
            let dir = MachineDir::at(root, &name);
            let ours = dir.check().is_ok() && dir.target_path().exists();
            let target = ours.then(|| dir.read_target().ok()).flatten();
            let gate: Option<Option<Gate>> = ours.then(|| dir.read_gate().ok().flatten());
            let leases = if ours { dir.leases() } else { Vec::new() };
            let state = match (listed, core_record) {
                (None, _) => "unknown".to_string(),
                (Some(_), None) => "absent".to_string(),
                (Some(_), Some(record)) => {
                    record["state"].as_str().unwrap_or("unknown").to_string()
                }
            };
            let herdr_link = core_record.is_some_and(|record| {
                record["via"] == true
                    && record["target"]
                        .as_str()
                        .is_some_and(|via| via.ends_with(&format!(" core-link {name}")))
            });
            StatusRow {
                gate: gate.as_ref().map(|gate| {
                    let act = newest_act(&leases, now);
                    match gate {
                        Some(gate) if gate.closes_at(now, act) => gate.class.as_str().to_string(),
                        _ if act.is_none() => "dormant".to_string(),
                        _ => "open".to_string(),
                    }
                }),
                gate_record: gate
                    .as_ref()
                    .and_then(|gate| gate.as_ref())
                    .map(|gate| gate.class.as_str().to_string()),
                gate_at: gate.flatten().map(|gate| gate.at),
                leases: leases
                    .iter()
                    .map(|lease| LeaseRow {
                        holder: lease.holder.as_str().to_string(),
                        expires_at: lease.expires_at,
                        input_at: lease.input_at,
                        in_force: lease.in_force(now),
                        dials: lease.dials,
                    })
                    .collect(),
                last_dial: ours
                    .then(|| dir.read_dial_record())
                    .flatten()
                    .map(|dial| dial.at),
                herdr_link,
                far_herdr: ours
                    .then(|| super::fleet::cached_far_herdr(&dir))
                    .flatten()
                    .map(|far| FarHerdrRow {
                        path: far.path,
                        version: far.version,
                    }),
                target: target.as_ref().map(|target| target.target.clone()),
                session: target.map(|target| target.session),
                state,
                name: name.to_string(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_machine::state::{tests::scratch, GateClass};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn add_parses_target_name_session_and_config() {
        let parsed = parse_add(&args(&["--core", "ac@ac-mbm5"])).expect("parse");
        assert_eq!(parsed.name.as_str(), "ac-mbm5");
        assert_eq!(parsed.session, "default");
        assert_eq!(parsed.ssh_config, None);
        let parsed = parse_add(&args(&[
            "--core",
            "vm1",
            "--name",
            "vm",
            "--remote-session",
            "herdr",
        ]))
        .expect("parse");
        assert_eq!(
            (parsed.name.as_str(), parsed.session.as_str()),
            ("vm", "herdr")
        );
        assert_eq!(
            parsed.command_line(),
            "herdr machine add --core vm1 --name vm --remote-session herdr"
        );
    }

    #[test]
    fn add_refuses_options_as_targets_and_odd_names() {
        for bad in [
            &["--core"][..],
            &["--core", "-oProxyCommand=x"],
            &["--core", "a b"],
            &["--core", "h", "--name", "a/b"],
            &["--core", "h", "--remote-session", "x'y"],
            &["--core", "h", "--ssh-config", "relative"],
            &["--core", "h", concat!("--", "install")],
            &["--core", "h", "--name", "a", "--name", "b"],
        ] {
            assert!(parse_add(&args(bad)).is_err(), "{bad:?} was accepted");
        }
    }

    #[test]
    fn default_names_are_the_host() {
        assert_eq!(default_name("ac@ac-mbm5"), "ac-mbm5");
        assert_eq!(default_name("m4m"), "m4m");
        assert_eq!(default_name("ssh://u@host:2222/"), "host");
        assert_eq!(default_name("ssh://[::1]:22"), "::1");
    }

    #[test]
    fn the_via_quotes_the_herdr_path() {
        assert_eq!(
            sh_quote("/opt/homebrew/bin/herdr-beta"),
            "/opt/homebrew/bin/herdr-beta"
        );
        assert_eq!(sh_quote("/Apps/My herdr/herdr"), "'/Apps/My herdr/herdr'");
        assert_eq!(sh_quote("/a'b"), r"'/a'\''b'");
    }

    // V22's notice part (D8): a saved machine and a core machine on one host
    // are one pair; other hosts are not.
    #[test]
    fn saved_and_core_machines_on_one_host_pair_up() {
        let core = |name: &str, target: Option<&str>| LinkedMachine {
            name: MachineName::parse(name).unwrap(),
            gate: "open".into(),
            target: target.map(str::to_string),
        };
        let saved = vec![
            ("mbm5".to_string(), "ac@AC-MBM5".to_string()),
            ("vm".to_string(), "vm1".to_string()),
        ];
        let pairs = same_host_pairs(
            &saved,
            &[
                core("ac-mbm5", Some("ac-mbm5")),
                core("lab", Some("ac@lab")),
                core("byhand", None),
            ],
        );
        assert_eq!(
            pairs,
            [("mbm5".to_string(), MachineName::parse("ac-mbm5").unwrap())]
        );
    }

    #[test]
    fn status_rows_merge_core_and_state() {
        let root = scratch("status");
        let ours = MachineDir::at(&root, &MachineName::parse("loop").expect("name"));
        ours.create().expect("create");
        ours.write_target(&Target {
            target: "ac@loop".into(),
            session: "far".into(),
            ssh: "/usr/bin/ssh".into(),
            ssh_config: None,
            ssh_auth_sock: None,
        })
        .expect("target");
        ours.write_gate(&Gate {
            class: GateClass::AuthFailed,
            at: 10.0,
            detail: None,
        })
        .expect("gate");
        ours.write_lease(&LeaseHolder::manual(), 2_000.0, 5.0)
            .expect("lease");
        let listed = json!({"machines": [
            {"name": "loop", "state": "unreachable", "via": true, "target": "/bin/herdr core-link loop"},
            {"name": "vps", "state": "connected", "via": false, "target": "vps"},
        ]});
        let rows = status_rows(&root, Some(&listed), 1_000.0);
        let json = serde_json::to_value(&rows).expect("json");
        assert_eq!(json[0]["name"], "loop");
        assert_eq!(json[0]["state"], "unreachable");
        assert_eq!(json[0]["gate"], "auth_failed");
        assert_eq!(json[0]["herdr_link"], true);
        assert_eq!(json[0]["leases"][0]["holder"], "manual");
        assert_eq!(json[0]["leases"][0]["in_force"], true);
        assert_eq!(json[1]["name"], "vps");
        assert_eq!(json[1]["gate"], Value::Null);
        assert_eq!(json[1]["herdr_link"], false);

        let down = status_rows(&root, None, 1_000.0);
        assert_eq!(down.len(), 1);
        assert_eq!(down[0].state, "unknown");

        // No lease in force is `dormant`; a first failure closes nothing;
        // `retry_spent` stands until a newer act.
        ours.clear_gate().expect("clear");
        assert_eq!(
            status_rows(&root, None, 2_000.0)[0].gate.as_deref(),
            Some("dormant")
        );
        assert_eq!(
            status_rows(&root, None, 1_000.0)[0].gate.as_deref(),
            Some("open")
        );
        ours.write_gate(&Gate {
            class: GateClass::DialFailed,
            at: 10.0,
            detail: None,
        })
        .expect("gate");
        let row = &status_rows(&root, None, 1_000.0)[0];
        assert_eq!(
            (row.gate.as_deref(), row.gate_record.as_deref()),
            (Some("open"), Some("dial_failed"))
        );
        ours.write_gate(&Gate {
            class: GateClass::RetrySpent,
            at: 10.0,
            detail: None,
        })
        .expect("gate");
        assert_eq!(
            status_rows(&root, None, 1_000.0)[0].gate.as_deref(),
            Some("retry_spent")
        );
        ours.write_lease(&LeaseHolder::client(1), 2_000.0, 11.0)
            .expect("act");
        assert_eq!(
            status_rows(&root, None, 1_000.0)[0].gate.as_deref(),
            Some("open")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn wake_takes_a_name_and_a_bounded_time() {
        assert_eq!(
            parse_wake(&args(&["ac-mbm5"])).expect("parse"),
            (
                MachineName::parse("ac-mbm5").expect("name"),
                MANUAL_LEASE_SECS
            )
        );
        assert_eq!(
            parse_wake(&args(&["m", "--for", "90"])).expect("secs").1,
            90.0
        );
        assert_eq!(
            parse_wake(&args(&["m", "--for", "2h"])).expect("hours").1,
            7_200.0
        );
        assert_eq!(
            parse_wake(&args(&["--for", "12h", "m"])).expect("max").1,
            MAX_WAKE_SECS
        );
        for bad in [
            &[][..],
            &["m", "--for", "13h"],
            &["m", "--for", "0"],
            &["m", "--for", "5d"],
            &["m", "--for"],
            &["m", "n"],
            &["a/b"],
            &["m", "--dial"],
        ] {
            assert!(parse_wake(&args(bad)).is_err(), "{bad:?} was accepted");
        }
    }
}
