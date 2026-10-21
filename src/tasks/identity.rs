//! Who this server is for the task store: which host owns its tasks, which
//! session may talk to the bus, and which backend it runs.

use super::ids::HostId;
use super::model::TaskError;

/// The owner part used when no `[tasks] host_id` is set. Only the file
/// backend may run on it: the key would not mean the same on another Mac.
const LOCAL_HOST: &str = "local";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendKind {
    File,
    Kv,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskIdentity {
    pub host: HostId,
    /// The name of this server's session (`default` for the unnamed one).
    pub session: String,
    /// The session that runs the `kv` backend; empty when none is named.
    pub bus_session: String,
    pub backend: BackendKind,
}

impl TaskIdentity {
    pub(crate) fn from_config(config: &crate::config::TasksConfig, session: Option<&str>) -> Self {
        Self {
            host: HostId::new(&config.host_id)
                .unwrap_or_else(|| HostId::new(LOCAL_HOST).expect("a valid host id")),
            session: session
                .unwrap_or(crate::session::DEFAULT_SESSION_NAME)
                .to_owned(),
            bus_session: config.bus_session.clone(),
            backend: match config.backend {
                crate::config::TasksBackend::File => BackendKind::File,
                crate::config::TasksBackend::Kv => BackendKind::Kv,
            },
        }
    }

    /// Whether this server may run the `kv` backend and read the host's NATS
    /// credential. A server on the file backend needs no bus.
    pub(crate) fn may_use_bus(&self) -> bool {
        self.backend == BackendKind::File || self.session == self.bus_session
    }

    /// May this server write a task owned by `owner`? Refuses, without
    /// queueing anything, when the server is not the bus session or the task
    /// belongs to another host.
    pub(crate) fn check_write(&self, owner: &HostId) -> Result<(), TaskError> {
        if !self.may_use_bus() {
            return Err(TaskError::NotBusSession {
                bus_session: self.bus_session.clone(),
            });
        }
        if *owner != self.host {
            return Err(TaskError::OwnedElsewhere {
                owner_host: owner.to_string(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn test_identity(host: &str) -> TaskIdentity {
    TaskIdentity {
        host: HostId::new(host).unwrap(),
        session: "default".into(),
        bus_session: String::new(),
        backend: BackendKind::File,
    }
}
