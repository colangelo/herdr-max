//! The `kv` backend: tasks in a NATS JetStream KV bucket (`TASKS`) shared by
//! both Macs. Compiled only with the cargo feature `tasks-kv`; the default
//! build links no NATS crate.
//!
//! The store thread calls these methods and each one blocks on a private
//! runtime. A watch is forwarded by a task on that runtime into a plain
//! channel. Revisions are the bucket's own sequence numbers.

use async_nats::jetstream::kv::{
    CreateErrorKind, Entry, Operation, Store as Bucket, UpdateErrorKind,
};
use bytes::Bytes;
use futures_util::StreamExt;
use std::time::SystemTime;

use tokio::runtime::Runtime;

use super::ids::{Rev, TaskKey, WriteId};
use super::model::Task;
use super::store::{
    landed_before, Mirror, MirrorEvent, MirrorSink, StoreError, StoreKey, StoreResult, TaskStore,
    WatchEvent, WatchSink,
};

/// Where and as whom to connect.
#[derive(Clone)]
pub(crate) struct KvSettings {
    pub url: String,
    pub user: Option<String>,
    pub password: Option<String>,
    /// The JetStream domain (`hub` in the fleet), when there is one.
    pub domain: Option<String>,
    pub bucket: String,
    /// Worklink's issue mirror and the asks, which are read, never written.
    pub issues_bucket: String,
    pub asks_bucket: String,
    /// The prefix of reply inboxes. The fleet's herdr users may only receive
    /// replies on `_INBOX_herdr_<host>`.
    pub inbox_prefix: Option<String>,
}

impl std::fmt::Debug for KvSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KvSettings")
            .field("url", &super::bus_env::redact_url(&self.url))
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "<hidden>"))
            .field("domain", &self.domain)
            .field("bucket", &self.bucket)
            .field("issues_bucket", &self.issues_bucket)
            .field("asks_bucket", &self.asks_bucket)
            .field("inbox_prefix", &self.inbox_prefix)
            .finish()
    }
}

impl KvSettings {
    /// Settings from a credential file, for the host `host_id`.
    pub(crate) fn from_bus_env(env: super::bus_env::BusEnv, host_id: &str) -> Self {
        Self {
            url: env.url,
            user: Some(env.user),
            password: Some(env.password),
            domain: env.js_domain,
            bucket: env.bucket,
            issues_bucket: env.issues_bucket,
            asks_bucket: env.asks_bucket,
            inbox_prefix: Some(
                env.inbox_prefix
                    .unwrap_or_else(|| format!("_INBOX_herdr_{host_id}")),
            ),
        }
    }
}

pub(crate) struct KvStore {
    runtime: Runtime,
    bucket: Bucket,
    context: async_nats::jetstream::Context,
    issues_bucket: String,
    asks_bucket: String,
}

impl KvStore {
    /// Connect and open the bucket, which must already exist: infra creates it.
    pub(crate) fn connect(settings: &KvSettings) -> StoreResult<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("herdr-task-kv")
            .enable_all()
            .build()
            .map_err(|err| StoreError::Io(err.to_string()))?;
        let (bucket, context) = runtime.block_on(async {
            let mut options = async_nats::ConnectOptions::new();
            if let (Some(user), Some(password)) = (&settings.user, &settings.password) {
                options = options.user_and_password(user.clone(), password.clone());
            }
            if let Some(prefix) = &settings.inbox_prefix {
                options = options.custom_inbox_prefix(prefix);
            }
            let client = options.connect(&settings.url).await.map_err(unavailable)?;
            let context = match &settings.domain {
                Some(domain) => async_nats::jetstream::with_domain(client, domain),
                None => async_nats::jetstream::new(client),
            };
            let bucket = context
                .get_key_value(&settings.bucket)
                .await
                .map_err(unavailable)?;
            Ok::<_, StoreError>((bucket, context))
        })?;
        Ok(Self {
            runtime,
            bucket,
            context,
            issues_bucket: settings.issues_bucket.clone(),
            asks_bucket: settings.asks_bucket.clone(),
        })
    }

    fn entry(&self, key: &str) -> StoreResult<Option<(Rev, Vec<u8>)>> {
        Ok(self.entry_at(key)?.map(|(rev, value, _)| (rev, value)))
    }

    /// As [`Self::entry`], with the time the server took the revision.
    fn entry_at(&self, key: &str) -> StoreResult<Option<(Rev, Vec<u8>, SystemTime)>> {
        let entry = self
            .runtime
            .block_on(self.bucket.entry(key.to_owned()))
            .map_err(unavailable)?;
        Ok(entry.and_then(|entry| {
            let (rev, value) = live(&entry)?;
            Some((rev, value, entry.created.into()))
        }))
    }

    fn document(doc: &Task) -> StoreResult<Bytes> {
        serde_json::to_vec(doc)
            .map(Bytes::from)
            .map_err(|err| StoreError::Refused(err.to_string()))
    }
}

/// A read or a connection that failed: nothing was changed, so trying again
/// later is safe.
fn unavailable(err: impl std::fmt::Display) -> StoreError {
    StoreError::Unavailable(err.to_string())
}

/// What a failed create means. A publish that errored or timed out may have
/// been stored all the same, so only a bad key is a plain failure.
fn create_error(kind: CreateErrorKind, why: String) -> StoreError {
    match kind {
        CreateErrorKind::InvalidKey => StoreError::Refused(why),
        CreateErrorKind::AlreadyExists
        | CreateErrorKind::Publish
        | CreateErrorKind::Ack
        | CreateErrorKind::Other => StoreError::Indeterminate(why),
    }
}

/// What a failed update means; see [`create_error`].
fn update_error(kind: UpdateErrorKind, why: String) -> StoreError {
    match kind {
        UpdateErrorKind::InvalidKey => StoreError::Refused(why),
        UpdateErrorKind::WrongLastRevision | UpdateErrorKind::TimedOut | UpdateErrorKind::Other => {
            StoreError::Indeterminate(why)
        }
    }
}

/// The value of a put; a delete or purge marker reads as absent.
fn live(entry: &Entry) -> Option<(Rev, Vec<u8>)> {
    (entry.operation == Operation::Put).then(|| (Rev(entry.revision), entry.value.to_vec()))
}

impl TaskStore for KvStore {
    fn create(&mut self, key: &TaskKey, doc: &Task, write: WriteId) -> StoreResult<Rev> {
        let name = key.to_string();
        let value = Self::document(doc)?;
        match self.runtime.block_on(self.bucket.create(&name, value)) {
            Ok(revision) => Ok(Rev(revision)),
            Err(err) if err.kind() == CreateErrorKind::AlreadyExists => {
                match self.entry(&name)? {
                    Some((rev, current)) if landed_before(&current, &write) => Ok(rev),
                    Some(current) => Err(StoreError::AlreadyExists { current }),
                    // Removed between the two calls: the caller retries.
                    None => Err(StoreError::Unavailable("key vanished during create".into())),
                }
            }
            Err(err) => Err(create_error(err.kind(), err.to_string())),
        }
    }

    fn update(
        &mut self,
        key: &TaskKey,
        doc: &Task,
        expected: Rev,
        write: WriteId,
    ) -> StoreResult<Rev> {
        let name = key.to_string();
        // Revision 0 would make the bucket create the key; the file backend
        // calls that a conflict, and so does this one.
        if expected == Rev(0) {
            return Err(StoreError::Conflict {
                current: self.entry(&name)?,
            });
        }
        let value = Self::document(doc)?;
        match self
            .runtime
            .block_on(self.bucket.update(&name, value, expected.0))
        {
            Ok(revision) => Ok(Rev(revision)),
            Err(err) if err.kind() == UpdateErrorKind::WrongLastRevision => {
                match self.entry(&name)? {
                    Some((rev, current)) if landed_before(&current, &write) => Ok(rev),
                    current => Err(StoreError::Conflict { current }),
                }
            }
            Err(err) => Err(update_error(err.kind(), err.to_string())),
        }
    }

    fn get(&mut self, key: &StoreKey) -> StoreResult<Option<(Rev, Vec<u8>)>> {
        self.entry(&key.0)
    }

    fn scan(&mut self, prefix: &str) -> StoreResult<Vec<WatchEvent>> {
        let keys = self.runtime.block_on(async {
            let mut stream = self.bucket.keys().await.map_err(unavailable)?;
            let mut keys = Vec::new();
            while let Some(key) = stream.next().await {
                let key = key.map_err(unavailable)?;
                if key.starts_with(prefix) {
                    keys.push(key);
                }
            }
            Ok::<_, StoreError>(keys)
        })?;
        let mut found = Vec::new();
        for key in keys {
            if let Some((rev, value, at)) = self.entry_at(&key)? {
                found.push(WatchEvent {
                    key: StoreKey(key),
                    rev,
                    value: Some(value),
                    at: Some(at),
                });
            }
        }
        found.sort_by(|left, right| left.key.cmp(&right.key));
        Ok(found)
    }

    fn watch(&mut self, prefix: &str, sink: WatchSink) -> StoreResult<()> {
        let mut watch = self
            .runtime
            .block_on(async {
                if prefix.is_empty() {
                    self.bucket.watch_all().await
                } else {
                    self.bucket.watch(format!("{prefix}>")).await
                }
            })
            .map_err(unavailable)?;
        self.runtime.spawn(async move {
            while let Some(item) = watch.next().await {
                let Ok(entry) = item else { continue };
                let event = WatchEvent {
                    key: StoreKey(entry.key.clone()),
                    rev: Rev(entry.revision),
                    value: live(&entry).map(|(_, value)| value),
                    at: Some(entry.created.into()),
                };
                if !sink(event) {
                    break;
                }
            }
        });
        Ok(())
    }

    fn mirror(&mut self, which: Mirror, sink: MirrorSink) -> StoreResult<bool> {
        let name = match which {
            Mirror::Issues => self.issues_bucket.clone(),
            Mirror::Asks => self.asks_bucket.clone(),
        };
        let context = self.context.clone();
        // An ordered consumer that starts at the last value of every key and
        // then follows the changes: no scan, no key list, nothing that waits
        // for one answer per key. (No durable consumer; the server may be
        // configured to refuse one.)
        let (empty, mut watch) = self.runtime.block_on(async {
            let bucket = context.get_key_value(&name).await.map_err(unavailable)?;
            let empty = bucket.status().await.map_err(unavailable)?.values() == 0;
            let watch = bucket.watch_with_history(">").await.map_err(unavailable)?;
            Ok::<_, StoreError>((empty, watch))
        })?;
        self.runtime.spawn(async move {
            // An empty bucket sends no entry to say the start is over.
            if empty && !sink(MirrorEvent::Current) {
                return;
            }
            let mut told = empty;
            let mut failures = 0u32;
            loop {
                let item = match watch.next().await {
                    Some(item) => item,
                    None => {
                        let _ = sink(MirrorEvent::Lost("the watch ended".to_owned()));
                        return;
                    }
                };
                let entry = match item {
                    Ok(entry) => {
                        failures = 0;
                        entry
                    }
                    Err(err) => {
                        // The consumer re-creates itself on most errors; a run
                        // of them means it cannot.
                        failures += 1;
                        if failures >= 10 {
                            let _ = sink(MirrorEvent::Lost(format!("the watch failed: {err}")));
                            return;
                        }
                        continue;
                    }
                };
                let current = entry.seen_current;
                let event = WatchEvent {
                    key: StoreKey(entry.key.clone()),
                    rev: Rev(entry.revision),
                    value: live(&entry).map(|(_, value)| value),
                    at: Some(entry.created.into()),
                };
                if !sink(MirrorEvent::Entry(event)) {
                    return;
                }
                if current && !told {
                    told = true;
                    if !sink(MirrorEvent::Current) {
                        return;
                    }
                }
            }
        });
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    use super::*;

    /// A throwaway `nats-server -js` that is killed when this drops.
    struct Server {
        child: Child,
        url: String,
        dir: std::path::PathBuf,
    }

    impl Server {
        /// `None` when `nats-server` is not installed: the test skips, since the
        /// feature build is opt-in and not every machine has the server.
        fn start(auth: Option<(&str, &str)>) -> Option<Self> {
            let port = TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            let dir = std::env::temp_dir().join(format!(
                "herdr-nats-test-{}-{}",
                std::process::id(),
                crate::ulid::new_ulid()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let mut command = Command::new("nats-server");
            command
                .args(["-js", "-a", "127.0.0.1", "-p", &port.to_string(), "-sd"])
                .arg(&dir)
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if let Some((user, password)) = auth {
                command.args(["--user", user, "--pass", password]);
            }
            let child = match command.spawn() {
                Ok(child) => child,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    assert!(
                        std::env::var_os("HERDR_REQUIRE_NATS").is_none(),
                        "HERDR_REQUIRE_NATS is set but nats-server is not on PATH"
                    );
                    eprintln!("skipped: nats-server is not on PATH");
                    let _ = std::fs::remove_dir_all(&dir);
                    return None;
                }
                Err(err) => panic!("cannot start nats-server: {err}"),
            };
            let server = Self {
                child,
                url: format!("127.0.0.1:{port}"),
                dir,
            };
            server.wait_for_jetstream(auth);
            Some(server)
        }

        /// Ready means JetStream answers, not just that the port accepts.
        fn wait_for_jetstream(&self, auth: Option<(&str, &str)>) {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                let ready = runtime.block_on(async {
                    let mut options = async_nats::ConnectOptions::new();
                    if let Some((user, password)) = auth {
                        options = options.user_and_password(user.into(), password.into());
                    }
                    let Ok(client) = options.connect(&self.url).await else {
                        return false;
                    };
                    async_nats::jetstream::new(client)
                        .query_account()
                        .await
                        .is_ok()
                });
                if ready {
                    return;
                }
                assert!(Instant::now() < deadline, "JetStream did not come up");
                std::thread::sleep(Duration::from_millis(100));
            }
        }

        fn create_bucket(&self, auth: Option<(&str, &str)>) {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                let mut options = async_nats::ConnectOptions::new();
                if let Some((user, password)) = auth {
                    options = options.user_and_password(user.into(), password.into());
                }
                let client = options.connect(&self.url).await.unwrap();
                async_nats::jetstream::new(client)
                    .create_key_value(async_nats::jetstream::kv::Config {
                        bucket: "TASKS".into(),
                        history: 5,
                        ..Default::default()
                    })
                    .await
                    .unwrap();
            });
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn settings(url: &str) -> KvSettings {
        KvSettings {
            url: url.to_owned(),
            user: None,
            password: None,
            domain: None,
            bucket: "TASKS".into(),
            issues_bucket: "ISSUES".into(),
            asks_bucket: "ASKS".into(),
            inbox_prefix: None,
        }
    }

    #[test]
    fn kv_backend_conformance() {
        let Some(server) = Server::start(None) else {
            return;
        };
        server.create_bucket(None);
        let mut store =
            KvStore::connect(&settings(&server.url)).expect("connect to the throwaway server");
        crate::tasks::conformance::run(&mut store);
    }

    /// Make `name` on the throwaway server and put `entries` in it.
    fn fill_bucket(server: &Server, name: &str, entries: &[(&str, &str)]) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let client = async_nats::connect(&server.url).await.unwrap();
            let kv = async_nats::jetstream::new(client)
                .create_key_value(async_nats::jetstream::kv::Config {
                    bucket: name.into(),
                    history: 3,
                    ..Default::default()
                })
                .await
                .unwrap();
            for (key, value) in entries {
                kv.put(*key, value.to_string().into()).await.unwrap();
            }
        });
    }

    fn events_until_current(received: &std::sync::mpsc::Receiver<MirrorEvent>) -> Vec<WatchEvent> {
        let mut entries = Vec::new();
        loop {
            match received.recv_timeout(Duration::from_secs(10)).unwrap() {
                MirrorEvent::Entry(event) => entries.push(event),
                MirrorEvent::Current => return entries,
                MirrorEvent::Lost(why) => panic!("the watch was lost: {why}"),
            }
        }
    }

    #[test]
    fn a_mirror_delivers_every_last_value_then_current_then_the_changes() {
        let Some(server) = Server::start(None) else {
            return;
        };
        server.create_bucket(None);
        // `a` was written twice; only its last value is delivered. `gone` is
        // deleted, and the delete marker is delivered as an absent value.
        fill_bucket(
            &server,
            "ASKS",
            &[
                ("g.a", "{\"status\":\"open\"}"),
                ("g.a", "{\"status\":\"answered\"}"),
            ],
        );
        fill_bucket(&server, "ISSUES", &[]);
        let mut store = KvStore::connect(&settings(&server.url)).unwrap();

        let (sink_tx, received) = std::sync::mpsc::channel();
        let sink_tx = std::sync::Mutex::new(sink_tx);
        assert!(store
            .mirror(
                Mirror::Asks,
                Box::new(move |event| sink_tx.lock().unwrap().send(event).is_ok())
            )
            .unwrap());
        let start = events_until_current(&received);
        assert_eq!(start.len(), 1, "{start:?}");
        assert_eq!(start[0].key.0, "g.a");
        assert_eq!(
            start[0].value.as_deref(),
            Some(&b"{\"status\":\"answered\"}"[..])
        );

        // An empty bucket says it is current without any entry.
        let (empty_tx, empty_rx) = std::sync::mpsc::channel();
        let empty_tx = std::sync::Mutex::new(empty_tx);
        assert!(store
            .mirror(
                Mirror::Issues,
                Box::new(move |event| empty_tx.lock().unwrap().send(event).is_ok())
            )
            .unwrap());
        assert!(events_until_current(&empty_rx).is_empty());

        // Then the changes: a put, and a delete as an absent value.
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let client = async_nats::connect(&server.url).await.unwrap();
            let kv = async_nats::jetstream::new(client)
                .get_key_value("ASKS")
                .await
                .unwrap();
            kv.put("g.b", "{\"status\":\"open\"}".into()).await.unwrap();
            kv.delete("g.a").await.unwrap();
        });
        let mut seen = Vec::new();
        while seen.len() < 2 {
            match received.recv_timeout(Duration::from_secs(10)).unwrap() {
                MirrorEvent::Entry(event) => seen.push((event.key.0, event.value.is_some())),
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(seen, [("g.b".to_owned(), true), ("g.a".to_owned(), false)]);
    }

    #[test]
    fn kv_connects_with_the_user_password_and_inbox_from_a_bus_env_file() {
        let Some(server) = Server::start(Some(("herdr-m4m", "hunter2"))) else {
            return;
        };
        server.create_bucket(Some(("herdr-m4m", "hunter2")));
        let env = crate::tasks::bus_env::BusEnv {
            url: server.url.clone(),
            user: "herdr-m4m".into(),
            password: "hunter2".into(),
            js_domain: None,
            bucket: "TASKS".into(),
            issues_bucket: "ISSUES".into(),
            asks_bucket: "ASKS".into(),
            inbox_prefix: None,
        };
        let settings = KvSettings::from_bus_env(env.clone(), "m4m");
        assert_eq!(settings.inbox_prefix.as_deref(), Some("_INBOX_herdr_m4m"));
        assert!(!format!("{settings:?}").contains("hunter2"));
        let mut store = KvStore::connect(&settings).expect("authenticated connect");
        crate::tasks::conformance::run(&mut store);

        // A wrong password is a failed connection, not a panic.
        let wrong = KvSettings {
            password: Some("wrong".into()),
            ..settings
        };
        assert!(matches!(
            KvStore::connect(&wrong),
            Err(StoreError::Unavailable(_))
        ));

        // The file's own inbox prefix wins over the default.
        let named = KvSettings::from_bus_env(
            crate::tasks::bus_env::BusEnv {
                inbox_prefix: Some("_INBOX_other".into()),
                ..env
            },
            "m4m",
        );
        assert_eq!(named.inbox_prefix.as_deref(), Some("_INBOX_other"));
    }

    #[test]
    fn connecting_to_a_missing_server_is_unavailable_not_a_panic() {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let result = KvStore::connect(&settings(&format!("127.0.0.1:{port}")));
        assert!(matches!(result, Err(StoreError::Unavailable(_))));
    }

    #[test]
    fn only_a_bad_key_is_a_plain_failure_and_a_publish_error_may_have_landed() {
        for kind in [
            CreateErrorKind::Publish,
            CreateErrorKind::Ack,
            CreateErrorKind::Other,
        ] {
            assert!(matches!(
                create_error(kind, "x".into()),
                StoreError::Indeterminate(_)
            ));
        }
        assert!(matches!(
            create_error(CreateErrorKind::InvalidKey, "x".into()),
            StoreError::Refused(_)
        ));
        for kind in [UpdateErrorKind::TimedOut, UpdateErrorKind::Other] {
            assert!(matches!(
                update_error(kind, "x".into()),
                StoreError::Indeterminate(_)
            ));
        }
        assert!(matches!(
            update_error(UpdateErrorKind::InvalidKey, "x".into()),
            StoreError::Refused(_)
        ));
    }
}
