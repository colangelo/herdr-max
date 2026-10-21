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
use tokio::runtime::Runtime;

use super::ids::{Rev, TaskKey, WriteId};
use super::model::Task;
use super::store::{
    landed_before, StoreError, StoreKey, StoreResult, TaskStore, WatchEvent, WatchStream,
};

/// Where and as whom to connect.
#[derive(Debug, Clone)]
pub(crate) struct KvSettings {
    pub url: String,
    pub user: Option<String>,
    pub password: Option<String>,
    /// The JetStream domain (`hub` in the fleet), when there is one.
    pub domain: Option<String>,
    pub bucket: String,
}

pub(crate) struct KvStore {
    runtime: Runtime,
    bucket: Bucket,
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
        let bucket = runtime.block_on(async {
            let mut options = async_nats::ConnectOptions::new();
            if let (Some(user), Some(password)) = (&settings.user, &settings.password) {
                options = options.user_and_password(user.clone(), password.clone());
            }
            let client = options.connect(&settings.url).await.map_err(unavailable)?;
            let context = match &settings.domain {
                Some(domain) => async_nats::jetstream::with_domain(client, domain),
                None => async_nats::jetstream::new(client),
            };
            context
                .get_key_value(&settings.bucket)
                .await
                .map_err(unavailable)
        })?;
        Ok(Self { runtime, bucket })
    }

    fn entry(&self, key: &str) -> StoreResult<Option<(Rev, Vec<u8>)>> {
        let entry = self
            .runtime
            .block_on(self.bucket.entry(key.to_owned()))
            .map_err(unavailable)?;
        Ok(entry.and_then(|entry| live(&entry)))
    }

    fn document(doc: &Task) -> StoreResult<Bytes> {
        serde_json::to_vec(doc)
            .map(Bytes::from)
            .map_err(|err| StoreError::Io(err.to_string()))
    }
}

fn unavailable(err: impl std::fmt::Display) -> StoreError {
    StoreError::Unavailable(err.to_string())
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
            Err(err) => Err(unavailable(err)),
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
            Err(err) => Err(unavailable(err)),
        }
    }

    fn get(&mut self, key: &StoreKey) -> StoreResult<Option<(Rev, Vec<u8>)>> {
        self.entry(&key.0)
    }

    fn scan(&mut self, prefix: &str) -> StoreResult<Vec<(StoreKey, Rev, Vec<u8>)>> {
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
            if let Some((rev, value)) = self.entry(&key)? {
                found.push((StoreKey(key), rev, value));
            }
        }
        found.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(found)
    }

    fn watch(&mut self, prefix: &str) -> StoreResult<WatchStream> {
        let subject = format!("{prefix}>");
        let mut watch = self
            .runtime
            .block_on(async {
                if prefix.is_empty() {
                    self.bucket.watch_all().await
                } else {
                    self.bucket.watch(subject).await
                }
            })
            .map_err(unavailable)?;
        let (sender, receiver) = std::sync::mpsc::channel();
        self.runtime.spawn(async move {
            while let Some(item) = watch.next().await {
                let Ok(entry) = item else { continue };
                let event = WatchEvent {
                    key: StoreKey(entry.key.clone()),
                    rev: Rev(entry.revision),
                    value: live(&entry).map(|(_, value)| value),
                };
                if sender.send(event).is_err() {
                    break;
                }
            }
        });
        Ok(receiver)
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
        fn start() -> Self {
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
            let child = Command::new("nats-server")
                .args(["-js", "-a", "127.0.0.1", "-p", &port.to_string(), "-sd"])
                .arg(&dir)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("the kv conformance test needs `nats-server` on PATH");
            let url = format!("127.0.0.1:{port}");
            let deadline = Instant::now() + Duration::from_secs(15);
            while std::net::TcpStream::connect(&url).is_err() {
                assert!(Instant::now() < deadline, "nats-server did not start");
                std::thread::sleep(Duration::from_millis(50));
            }
            Self { child, url, dir }
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn kv_backend_conformance() {
        let server = Server::start();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let client = async_nats::connect(&server.url).await.unwrap();
            async_nats::jetstream::new(client)
                .create_key_value(async_nats::jetstream::kv::Config {
                    bucket: "TASKS".into(),
                    history: 5,
                    ..Default::default()
                })
                .await
                .unwrap();
        });
        let mut store = KvStore::connect(&KvSettings {
            url: server.url.clone(),
            user: None,
            password: None,
            domain: None,
            bucket: "TASKS".into(),
        })
        .expect("connect to the throwaway server");
        crate::tasks::conformance::run(&mut store);
    }

    #[test]
    fn connecting_to_a_missing_server_is_unavailable_not_a_panic() {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let result = KvStore::connect(&KvSettings {
            url: format!("127.0.0.1:{port}"),
            user: None,
            password: None,
            domain: None,
            bucket: "TASKS".into(),
        });
        assert!(matches!(result, Err(StoreError::Unavailable(_))));
    }
}
