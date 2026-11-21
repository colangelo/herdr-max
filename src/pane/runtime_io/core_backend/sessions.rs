//! The herdr-core sessions herdr reaches, and its connections to them
//! (openspec/changes/core-phase3, D5 and D6).

use std::sync::{Arc, Mutex};

use tracing::warn;

use super::client::{self, CoreClient};
use super::{gave_up, lock, note_core_loss, primary, Connector};

/// The server's one connection to its primary core (D3), replaced when it
/// closes, and when the primary may be started again (core-phase3 D3).
static PRIMARY: Mutex<Primary> = Mutex::new(Primary {
    client: None,
    rule: primary::StartRule::new(),
});

struct Primary {
    client: Option<CoreClient>,
    rule: primary::StartRule,
}

/// The core session for this herdr session (D2): `herdr` (`herdrdev` for a
/// debug build), and `-<name>` for a named session, so a throwaway or test
/// herdr never shares a core with ac's sessions or touches the default core.
/// No release name can equal a debug one: release names never start with
/// `herdrdev`.
pub(crate) fn core_session_name() -> String {
    session_name_for(
        crate::session::active_name().as_deref(),
        cfg!(debug_assertions),
    )
}

fn session_name_for(herdr_session: Option<&str>, debug_build: bool) -> String {
    let mut name = String::from(if debug_build { "herdrdev" } else { "herdr" });
    if let Some(session) = herdr_session {
        name.push('-');
        name.push_str(session);
    }
    name
}

/// The server's connector: the live connection, or a new one. It starts the
/// primary core before the server's first connection, and later when its
/// socket has been missing or refusing for 3 s (core-phase3 D3).
pub(crate) fn server_connector() -> Connector {
    connector(true)
}

/// Like `server_connector`, but never starts a core before the server's
/// first connection: one that is not running then holds no terminal to
/// reattach to (D10). A primary that went down is started by the same 3 s
/// rule, so a pane reattached by a restore also comes back (core-phase3 D3).
pub(crate) fn existing_core_connector() -> Connector {
    connector(false)
}

/// A connector to `session` that never starts a core: the primary's shared
/// connection, or for any other session a connection of its own at that
/// session's own socket (core-phase3 D1b, D6).
pub(crate) fn existing_session_connector(session: &str) -> Connector {
    if session == core_session_name() {
        return existing_core_connector();
    }
    let session = session.to_string();
    Arc::new(move || client::connect_secondary(&session))
}

/// The core sessions a restore looks in for a pane's terminal before it
/// starts the pane fresh, none of which it starts (core-phase3 D1b): the
/// primary, the link's own, `herdr`, and `default` for the release default
/// herdr session only.
pub(crate) fn known_sessions(link_session: &str) -> Vec<String> {
    known_sessions_for(
        &core_session_name(),
        link_session,
        crate::session::active_name().is_none() && !cfg!(debug_assertions),
    )
}

fn known_sessions_for(primary: &str, link_session: &str, release_default: bool) -> Vec<String> {
    let mut sessions = vec![primary.to_string()];
    let mut add = |session: &str| {
        if !sessions.iter().any(|known| known == session) {
            sessions.push(session.to_string());
        }
    };
    add(link_session);
    if release_default {
        add("herdr");
        add("default");
    }
    sessions
}

fn connector(first_start: bool) -> Connector {
    Arc::new(move || {
        let mut primary = lock(&PRIMARY);
        if let Some(client) = primary.client.as_ref() {
            if !client.is_closed() {
                return Ok(client.clone());
            }
        }
        let session = core_session_name();
        let socket = client::socket_path(&session);
        // No events subscription: core's status comes only from bytes a
        // program prints, which any printed file can forge, so herdr does not
        // use it (security review M3; design D9). Detection and hooks decide.
        let err = match client::connect_at(&socket) {
            Ok((client, _)) => return Ok(primary.connected(client)),
            Err(err) => err,
        };
        let now = std::time::Instant::now();
        let down = client::socket_down(&socket);
        if !primary.rule.should_start(now, down, first_start, gave_up()) {
            return Err(err);
        }
        if primary.rule.after_loss() {
            if note_core_loss(now) {
                return Err(err);
            }
            warn!(
                %session,
                ?down,
                "herdr-core has been down for 3 s or more; starting it again"
            );
        }
        client::start_and_connect(&session, &socket).map(|client| primary.connected(client))
    })
}

impl Primary {
    fn connected(&mut self, client: CoreClient) -> CoreClient {
        self.rule.connected();
        self.client = Some(client.clone());
        client
    }
}

/// Terminals of `session` finished with `server_stop`: a person stopped
/// that core, so herdr does not start it until it answers again (core-phase3
/// D3b). Only the primary is ever started, so only its stop is recorded.
pub(crate) fn note_stopped_by_person(session: &str) {
    if session == core_session_name() {
        lock(&PRIMARY).rule.stopped_by_person();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::super::client::CoreError;
    use super::super::fake;
    use super::*;

    #[test]
    fn core_session_names_follow_design_d2() {
        assert_eq!(session_name_for(None, false), "herdr");
        assert_eq!(session_name_for(None, true), "herdrdev");
        assert_eq!(session_name_for(Some("trial"), false), "herdr-trial");
        assert_eq!(session_name_for(Some("trial"), true), "herdrdev-trial");
        // A release session named `dev` must not reach a debug build's core.
        assert_ne!(
            session_name_for(Some("dev"), false),
            session_name_for(None, true)
        );
    }

    static STARTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    static STARTED: Mutex<Option<fake::FakeCore>> = Mutex::new(None);
    static STARTED_AT: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);

    /// A start that returns before the core listens, as one that landed in a
    /// core restart and exited 5: the core comes up 300 ms later.
    fn start_late(_session: &str) -> Result<(), CoreError> {
        STARTS.fetch_add(1, Ordering::AcqRel);
        let socket = lock(&STARTED_AT).clone().expect("socket");
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            let _ = std::fs::remove_file(&socket);
            let dir = socket.parent().expect("dir").to_path_buf();
            *lock(&STARTED) = Some(fake::FakeCore::start_in(dir, "p.sock", |_| {}));
        });
        Ok(())
    }

    // V38, V15 in a unit (core-phase3 D3): after the first connection, a
    // primary whose socket stays and refuses (a SIGKILLed core) is started
    // after 3 s down, not before; a start whose core listens only later is
    // polled; a secondary session is never started.
    #[test]
    fn a_refusing_primary_is_started_after_three_seconds_and_a_secondary_never() {
        let first = fake::FakeCore::start();
        std::env::set_var("HC_SOCKET", &first.socket);
        let connect = server_connector();
        connect().expect("first connection");

        let dir = std::env::temp_dir().join(format!("hcr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .expect("mode");
        let socket = dir.join("p.sock");
        drop(std::os::unix::net::UnixListener::bind(&socket).expect("bind"));
        assert_eq!(
            client::socket_down(&socket),
            Some(primary::SocketDown::Refused),
            "the file stays and refuses"
        );
        *lock(&STARTED_AT) = Some(socket.clone());
        *lock(&client::TEST_CORE_START) = Some(start_late);
        std::env::set_var("HC_SOCKET", &socket);
        first.hang_up();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while lock(&PRIMARY)
            .client
            .as_ref()
            .is_some_and(|c| !c.is_closed())
        {
            assert!(
                std::time::Instant::now() < deadline,
                "connection not closed"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let down_at = std::time::Instant::now();
        while down_at.elapsed() < primary::DOWN_BEFORE_START - std::time::Duration::from_millis(200)
        {
            assert!(connect().is_err());
            assert_eq!(STARTS.load(Ordering::Acquire), 0, "no start before 3 s");
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
        connect().expect("started and polled until it answered");
        assert_eq!(STARTS.load(Ordering::Acquire), 1);

        std::env::set_var("XDG_RUNTIME_DIR", &dir);
        assert!(existing_session_connector("herdrdev-other")().is_err());
        assert_eq!(
            STARTS.load(Ordering::Acquire),
            1,
            "a secondary is never started"
        );
        *lock(&STARTED) = None;
        let _ = std::fs::remove_dir_all(&dir);
    }

    // D3b: a core whose terminals finished with `server_stop` is not
    // started while the stop stands.
    #[test]
    fn a_primary_a_person_stopped_is_not_started() {
        let first = fake::FakeCore::start();
        std::env::set_var("HC_SOCKET", &first.socket);
        server_connector()().expect("first connection");
        *lock(&client::TEST_CORE_START) = Some(start_late);
        note_stopped_by_person(&core_session_name());
        let missing = std::env::temp_dir().join(format!("hcs-{}.sock", std::process::id()));
        std::env::set_var("HC_SOCKET", &missing);
        first.hang_up();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while lock(&PRIMARY)
            .client
            .as_ref()
            .is_some_and(|c| !c.is_closed())
        {
            assert!(
                std::time::Instant::now() < deadline,
                "connection not closed"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let down_at = std::time::Instant::now();
        while down_at.elapsed() < primary::DOWN_BEFORE_START + std::time::Duration::from_millis(500)
        {
            assert!(server_connector()().is_err());
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        assert_eq!(STARTS.load(Ordering::Acquire), 0, "stopped by a person");
    }
}
