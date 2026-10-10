use crate::api::schema::{EmptyParams, Method, Request};

/// `herdr client list` reads the server's client table. Anything else after
/// `client` is the hidden client mode `main` starts, so it is not ours: `None`.
pub(super) fn run_client_command(args: &[String]) -> std::io::Result<Option<i32>> {
    match args.first().map(|arg| arg.as_str()) {
        Some("list") => client_list(&args[1..]).map(Some),
        _ => Ok(None),
    }
}

fn client_list(args: &[String]) -> std::io::Result<i32> {
    if !args.is_empty() {
        eprintln!("usage: herdr client list");
        return Ok(2);
    }
    super::print_response(&super::send_request(&Request {
        id: "cli:client:list".into(),
        method: Method::ClientList(EmptyParams::default()),
    })?)
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
