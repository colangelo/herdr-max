//! Unit tests must not read or write the developer's real herdr state. These
//! pin the rules that keep that true: a unit-test binary only honors a base
//! directory, `HOME` or socket path that lies in the scratch area, and ignores
//! the value the developer's shell exports for the machine's real one.

use std::ffi::OsString;
use std::path::PathBuf;

use super::*;

/// Sets or clears variables for one test and puts them back afterwards. Hold
/// [`test_config_env_lock`] for as long as it lives.
struct EnvRestore(Vec<(&'static str, Option<OsString>)>);

impl EnvRestore {
    fn new(vars: &[&'static str]) -> Self {
        Self(
            vars.iter()
                .map(|var| (*var, std::env::var_os(var)))
                .collect(),
        )
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        for (var, value) in self.0.drain(..) {
            match value {
                Some(value) => std::env::set_var(var, value),
                None => std::env::remove_var(var),
            }
        }
    }
}

// A path that is absolute on Unix only, so these rules are Unix tests.
#[cfg(unix)]
#[test]
fn config_and_state_dirs_ignore_a_real_xdg_home() {
    let _lock = test_config_env_lock().lock().unwrap();
    let _restore = EnvRestore::new(&["XDG_CONFIG_HOME", "XDG_STATE_HOME"]);

    std::env::set_var("XDG_CONFIG_HOME", "/Users/dev/.config");
    std::env::set_var("XDG_STATE_HOME", "/Users/dev/.local/state");
    assert_eq!(config_dir(), io::unit_test_host_dir("config"));
    assert_eq!(state_dir(), io::unit_test_host_dir("state"));

    std::env::remove_var("XDG_CONFIG_HOME");
    std::env::remove_var("XDG_STATE_HOME");
    assert_eq!(config_dir(), io::unit_test_host_dir("config"));
    assert_eq!(state_dir(), io::unit_test_host_dir("state"));

    let scratch = std::env::temp_dir().join("herdr-hermetic-xdg");
    std::env::set_var("XDG_CONFIG_HOME", &scratch);
    std::env::set_var("XDG_STATE_HOME", &scratch);
    assert_eq!(config_dir(), scratch.join(app_dir_name()));
    assert_eq!(state_dir(), scratch.join(app_dir_name()));
}

#[cfg(unix)]
#[test]
fn path_env_ignores_an_absolute_path_outside_the_scratch_area() {
    let _lock = test_config_env_lock().lock().unwrap();
    let _restore = EnvRestore::new(&[crate::api::SOCKET_PATH_ENV_VAR]);
    let var = crate::api::SOCKET_PATH_ENV_VAR;

    std::env::remove_var(var);
    assert_eq!(path_env(var), None, "unset stays unset");

    std::env::set_var(var, "/Users/dev/.config/herdr/herdr.sock");
    assert_eq!(path_env(var), None, "a real socket path is not honored");

    std::env::set_var(var, "/tmp/herdr-hermetic/herdr.sock");
    assert_eq!(
        path_env(var),
        Some(OsString::from("/tmp/herdr-hermetic/herdr.sock")),
        "a scratch path is honored"
    );

    std::env::set_var(var, "relative/herdr.sock");
    assert_eq!(
        path_env(var),
        Some(OsString::from("relative/herdr.sock")),
        "a relative path cannot leave the working directory"
    );
}

#[cfg(unix)]
#[test]
fn home_env_gives_a_real_home_a_private_stand_in() {
    let _lock = test_config_env_lock().lock().unwrap();
    let _restore = EnvRestore::new(&["HOME"]);

    std::env::set_var("HOME", "/Users/dev");
    assert_eq!(
        home_env(),
        Some(io::unit_test_host_dir("home").into_os_string())
    );

    let scratch = std::env::temp_dir().join("herdr-hermetic-home");
    std::env::set_var("HOME", &scratch);
    assert_eq!(home_env(), Some(scratch.into_os_string()));

    std::env::remove_var("HOME");
    assert_eq!(home_env(), None, "an unset HOME is still unset");
}

#[test]
fn the_stand_in_directories_are_private_to_the_process_and_apart() {
    let dirs: Vec<PathBuf> = ["config", "state", "home"]
        .into_iter()
        .map(io::unit_test_host_dir)
        .collect();
    for dir in &dirs {
        assert!(
            is_unit_test_scratch(dir),
            "{} must be scratch",
            dir.display()
        );
        assert!(dir
            .to_string_lossy()
            .contains(&std::process::id().to_string()));
    }
    assert_eq!(
        dirs.iter().collect::<std::collections::HashSet<_>>().len(),
        3
    );
}

#[cfg(unix)]
#[test]
fn isolated_host_env_is_a_private_host_and_restores_the_environment() {
    let before: Vec<_> = [
        crate::api::SOCKET_PATH_ENV_VAR,
        "XDG_CONFIG_HOME",
        crate::session::SESSION_ENV_VAR,
    ]
    .into_iter()
    .map(|var| (var, std::env::var_os(var)))
    .collect();

    {
        let host = IsolatedHostEnv::new("hermetic-self-test");
        assert_eq!(
            crate::session::active_api_socket_path(),
            host.dir().join("herdr.sock")
        );
        assert_eq!(
            crate::server::socket_paths::client_socket_path(),
            host.dir().join("herdr-client.sock")
        );
        assert_eq!(config_dir(), host.dir().join("config").join(app_dir_name()));
        assert_eq!(config_path(), config_dir().join("config.toml"));
        assert_eq!(
            crate::platform::ssh_agent::socket_path(),
            host.dir().join("herdr.sock.agent")
        );
    }

    let _lock = test_config_env_lock().lock().unwrap();
    for (var, value) in before {
        assert_eq!(std::env::var_os(var), value, "{var} must be restored");
    }
}
