use std::collections::HashMap;

pub(super) fn normalize_launch_env(
    env: HashMap<String, String>,
) -> Result<Vec<(String, String)>, (String, String)> {
    let mut normalized = Vec::with_capacity(env.len());
    for (key, value) in env {
        if key.is_empty() {
            return Err(("invalid_env".into(), "env key must not be empty".into()));
        }
        if key.contains('=') {
            return Err((
                "invalid_env".into(),
                format!("env key {key} must not contain '='"),
            ));
        }
        if key.contains('\0') {
            return Err((
                "invalid_env".into(),
                "env key must not contain NUL bytes".into(),
            ));
        }
        if value.contains('\0') {
            return Err((
                "invalid_env".into(),
                format!("env value for {key} must not contain NUL bytes"),
            ));
        }
        normalized.push((key, value));
    }
    normalized.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(normalized)
}

/// The reserved env key that names the run a pane is started for.
const RUN_ID_ENV: &str = "HERDR_RUN_ID";
const RUN_ID_MAX_BYTES: usize = 128;

/// The run id carried by a launch env, if it has a usable one. The key is still
/// passed to the shell; this only records it on the terminal.
pub(super) fn run_id_from_env(env: &[(String, String)]) -> Option<String> {
    env.iter()
        .find(|(key, _)| key == RUN_ID_ENV)
        .map(|(_, value)| value.as_str())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= RUN_ID_MAX_BYTES
                && !value.chars().any(char::is_control)
        })
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_launch_env_sorts_entries() {
        let env = HashMap::from([
            ("ZED".to_string(), "last".to_string()),
            ("ALPHA".to_string(), "first".to_string()),
        ]);

        assert_eq!(
            normalize_launch_env(env).unwrap(),
            vec![
                ("ALPHA".to_string(), "first".to_string()),
                ("ZED".to_string(), "last".to_string()),
            ]
        );
    }

    #[test]
    fn run_id_is_read_from_the_reserved_key_only_when_usable() {
        let env = |value: &str| vec![("HERDR_RUN_ID".to_string(), value.to_string())];
        assert_eq!(run_id_from_env(&env("run-1")).as_deref(), Some("run-1"));
        assert_eq!(run_id_from_env(&env("")), None);
        assert_eq!(run_id_from_env(&env("a\nb")), None);
        assert_eq!(run_id_from_env(&env(&"x".repeat(129))), None);
        assert_eq!(
            run_id_from_env(&[("OTHER".to_string(), "run-1".to_string())]),
            None
        );
    }

    #[test]
    fn normalize_launch_env_rejects_invalid_keys() {
        let env = HashMap::from([("BAD=KEY".to_string(), "value".to_string())]);

        assert_eq!(normalize_launch_env(env).unwrap_err().0, "invalid_env");
    }
}
