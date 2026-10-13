pub(super) fn run_popup_command(args: &[String]) -> std::io::Result<i32> {
    super::run_json_request(super::plugin::bindings::popup_request(args))
}

#[cfg(test)]
mod tests {
    fn parse_popup_close_args(args: &[String]) -> Result<Option<String>, String> {
        let request = crate::cli::plugin::bindings::popup_request(
            &[&["close".to_owned()][..], args].concat(),
        )
        .map_err(|error| format!("{error:?}"))?;
        match request.method {
            crate::api::schema::Method::PluginPopupClose(params) => Ok(Some(params.plugin_id)),
            _ => Ok(None),
        }
    }

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn close_without_options_targets_any_popup() {
        assert_eq!(parse_popup_close_args(&args(&[])), Ok(None));
    }

    #[test]
    fn close_plugin_accepts_both_spellings() {
        for values in [
            &["--plugin", "gestore.asks"][..],
            &["--plugin=gestore.asks"][..],
        ] {
            assert_eq!(
                parse_popup_close_args(&args(values)),
                Ok(Some("gestore.asks".to_owned()))
            );
        }
    }

    #[test]
    fn close_rejects_unknown_missing_and_repeated_options() {
        for values in [
            &["--plugin"][..],
            &["--force"][..],
            &["gestore.asks"][..],
            &["--plugin", "a", "--plugin", "b"][..],
        ] {
            assert!(
                parse_popup_close_args(&args(values)).is_err(),
                "{values:?} should be rejected"
            );
        }
    }
}
