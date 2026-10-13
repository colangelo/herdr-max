/// `herdr client list` reads the server's client table. Anything else after
/// `client` is the hidden client mode `main` starts, so it is not ours: `None`.
pub(super) fn run_client_command(args: &[String]) -> std::io::Result<Option<i32>> {
    match args.first().map(|arg| arg.as_str()) {
        Some("list") => client_list(&args[1..]).map(Some),
        _ => Ok(None),
    }
}

fn client_list(args: &[String]) -> std::io::Result<i32> {
    super::run_json_request(super::client_bindings::request(
        &[&["list".to_owned()][..], args].concat(),
    ))
}

#[cfg(test)]
mod tests {
    use super::run_client_command;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn anything_but_list_stays_with_the_hidden_client_mode() {
        for values in [&[][..], &["--remote-bridge"][..], &["lists"][..]] {
            assert!(
                matches!(run_client_command(&args(values)), Ok(None)),
                "{values:?} must fall through to the client mode"
            );
        }
    }

    #[test]
    fn list_rejects_extra_arguments_before_contacting_the_server() {
        assert!(matches!(
            run_client_command(&args(&["list", "--all"])),
            Ok(Some(2))
        ));
    }
}
