#[path = "tab_bindings.rs"]
mod bindings;

pub(crate) use bindings::command;

pub(super) fn run_tab_command(args: &[String]) -> std::io::Result<i32> {
    super::run_json_request(bindings::request(args))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::api::schema::{TabCreateParams, TabListParams};

    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn simple_families_tab_rejects_invalid_grammar_before_transport() {
        for values in [
            vec![],
            vec!["unknown"],
            vec!["list", "--workspace"],
            vec!["create", "--cwd"],
            vec!["create", "--label"],
            vec!["create", "--env"],
            vec!["create", "--env", "NO_EQUALS"],
            vec!["create", "--focus=true"],
            vec!["get"],
            vec!["get", "2", "extra"],
            vec!["focus"],
            vec!["focus", "2", "extra"],
            vec!["rename", "2"],
            vec!["close"],
            vec!["close", "2", "extra"],
        ] {
            assert!(
                bindings::request(&args(&values)).is_err(),
                "argv: {values:?}"
            );
        }
    }

    #[test]
    fn simple_families_tab_dto_literal_defaults() {
        let create: TabCreateParams = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(
            create,
            TabCreateParams {
                workspace_id: None,
                cwd: None,
                focus: false,
                label: None,
                env: HashMap::new(),
            }
        );
        assert_eq!(
            serde_json::to_value(create).unwrap(),
            serde_json::json!({"focus": false})
        );
        assert_eq!(
            serde_json::to_value(TabListParams::default()).unwrap(),
            serde_json::json!({})
        );
    }
}

#[cfg(test)]
mod fork_sync_tests {
    use super::*;
    fn parse_tab_sync_args(
        args: &[String],
    ) -> Result<crate::api::schema::TabSyncParams, super::super::projection::ParseError> {
        let request = bindings::request(&[&["sync".to_owned()][..], args].concat())?;
        let crate::api::schema::Method::TabSync(params) = request.method else {
            panic!("sync");
        };
        Ok(params)
    }
    use crate::api::schema::SyncMode;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn tab_sync_args_default_to_the_current_tab_toggle() {
        let params = parse_tab_sync_args(&args(&[])).unwrap();
        assert_eq!(params.tab_id, None);
        assert_eq!(params.mode, SyncMode::Toggle);
    }

    #[test]
    fn tab_sync_args_take_a_tab_and_a_mode() {
        let params = parse_tab_sync_args(&args(&["w_1:2", "--on"])).unwrap();
        assert_eq!(params.tab_id.as_deref(), Some("w_1:2"));
        assert_eq!(params.mode, SyncMode::On);
        let params = parse_tab_sync_args(&args(&["--tab", "w_1:2", "--off"])).unwrap();
        assert_eq!(params.mode, SyncMode::Off);
        assert!(parse_tab_sync_args(&args(&["--on", "--off"])).is_err());
        assert!(parse_tab_sync_args(&args(&["--bogus"])).is_err());
    }
}
