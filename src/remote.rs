mod args;
mod attach;
mod host;
mod process;
mod restart_policy;
mod saved;
#[cfg(unix)]
mod ssh_agent;

pub(crate) use args::*;
pub(crate) use attach::*;
pub(crate) use host::run_remote_client_bridge;
pub(crate) use saved::*;

pub(crate) fn run_remote_api_bridge(args: &[String]) -> std::io::Result<()> {
    match args {
        [] => {
            let path = crate::api::socket_path();
            let stream = crate::ipc::connect_local_stream(&path).map_err(|error| {
                std::io::Error::new(
                    error.kind(),
                    format!(
                        "failed to connect to remote Herdr API socket {}: {error}",
                        path.display()
                    ),
                )
            })?;
            crate::platform::forward_remote_bridge_stdio(stream, false)
        }
        [flag] if flag == "--check" => {
            println!("herdr-api-bridge-v1");
            Ok(())
        }
        [mode] if mode == "--request" => {
            let path = crate::api::socket_path();
            let stream = crate::ipc::connect_local_stream(&path).map_err(|error| {
                std::io::Error::new(
                    error.kind(),
                    format!(
                        "failed to connect to remote Herdr API socket {}: {error}",
                        path.display()
                    ),
                )
            })?;
            forward_one_api_request(std::io::stdin().lock(), stream, std::io::stdout().lock())
        }
        [mode, flag] if mode == "--request" && flag == "--check" => {
            println!("{API_REQUEST_BRIDGE}");
            Ok(())
        }
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "usage: herdr remote-api-bridge [--check | --request [--check]]",
        )),
    }
}

/// What `remote-api-bridge --request --check` prints.
pub(crate) const API_REQUEST_BRIDGE: &str = "herdr-api-request-v1";

/// The most `remote-api-bridge --request` reads as its request: 8 MiB,
/// what a Herdr Core `machine.exec` takes as input.
const API_REQUEST_LIMIT: u64 = 8 * 1024 * 1024;

/// `remote-api-bridge --request`: the request on `input`, read to its end,
/// goes to the API; the reply line goes to `output`. Unlike the stream
/// bridge it never half-closes the API connection, which would end a
/// request still waiting (`pane wait-output`, `events.wait`): a Herdr Core
/// `machine.exec` closes the program's stdin once the request is in
/// (core-phase5 D7).
fn forward_one_api_request(
    input: impl std::io::Read,
    mut api: impl std::io::Read + std::io::Write,
    mut output: impl std::io::Write,
) -> std::io::Result<()> {
    use std::io::{BufRead as _, Read as _};
    let mut request = Vec::new();
    input
        .take(API_REQUEST_LIMIT + 1)
        .read_to_end(&mut request)?;
    if request.len() as u64 > API_REQUEST_LIMIT {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the request is over 8 MiB",
        ));
    }
    api.write_all(&request)?;
    api.flush()?;
    let mut reply = Vec::new();
    std::io::BufReader::new(api).read_until(b'\n', &mut reply)?;
    output.write_all(&reply)?;
    output.flush()
}

pub(crate) fn print_saved_ssh_error_hint(err: &std::io::Error, target: &str) {
    if is_remote_host_key_error(err) {
        eprintln!(
            "hint: saved machines use strict host-key checking; add the host key to the configured known_hosts file, then retry."
        );
    } else {
        print_remote_error_hint(err, target);
    }
}

pub(crate) fn print_remote_error_hint(err: &std::io::Error, target: &str) {
    if is_remote_auth_error(err) {
        eprintln!(
            "hint: verify SSH access first with `{}`.",
            ssh_check_command(target)
        );
        eprintln!(
            "hint: if your SSH key has a passphrase, load it into ssh-agent with `ssh-add` before running `herdr --remote`."
        );
    }
}

fn is_remote_host_key_error(err: &std::io::Error) -> bool {
    let message = err.to_string().to_ascii_lowercase();
    message.contains("host key verification failed")
        || message.contains("remote host identification has changed")
}

fn is_remote_auth_error(err: &std::io::Error) -> bool {
    let message = err.to_string();
    message.contains("Permission denied")
        && (message.contains("(publickey")
            || message.contains("(keyboard-interactive")
            || message.contains("(password"))
}

fn ssh_check_command(target: &str) -> String {
    format!("ssh {}", shell_quote(target))
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value.chars().all(|ch| {
            ch.is_ascii_alphanumeric()
                || matches!(
                    ch,
                    '@' | '%' | '_' | '+' | '=' | ':' | ',' | '.' | '/' | '-'
                )
        })
    {
        return value.to_string();
    }

    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// core-phase5 D7: the one-request bridge keeps the API connection's
    /// write side open while the request waits, so the end of its input
    /// does not end the request; the reply line is passed on untouched.
    #[cfg(unix)]
    #[test]
    fn the_request_bridge_never_half_closes_while_the_request_waits() {
        use std::io::{BufRead as _, Read as _, Write as _};
        let (bridge_end, mut api_end) = std::os::unix::net::UnixStream::pair().expect("pair");
        let server = std::thread::spawn(move || {
            let mut line = Vec::new();
            std::io::BufReader::new(api_end.try_clone().expect("clone"))
                .read_until(b'\n', &mut line)
                .expect("request");
            // A waiting request: the client must still be there.
            api_end
                .set_read_timeout(Some(std::time::Duration::from_millis(300)))
                .expect("timeout");
            let mut byte = [0_u8; 1];
            let half_closed = matches!(api_end.read(&mut byte), Ok(0));
            api_end
                .write_all(b"{\"id\":\"w\",\"result\":{}}\n")
                .expect("reply");
            (line, half_closed)
        });
        let mut output = Vec::new();
        forward_one_api_request(
            &b"{\"id\":\"w\",\"method\":\"ping\"}\n"[..],
            bridge_end,
            &mut output,
        )
        .expect("forward");
        let (line, half_closed) = server.join().expect("server");
        assert_eq!(line, b"{\"id\":\"w\",\"method\":\"ping\"}\n");
        assert!(!half_closed, "the bridge half-closed the API connection");
        assert_eq!(output, b"{\"id\":\"w\",\"result\":{}}\n");
    }

    #[test]
    fn remote_host_key_error_matches_ssh_diagnostics() {
        for message in [
            "Host key verification failed.",
            "REMOTE HOST IDENTIFICATION HAS CHANGED!",
        ] {
            assert!(is_remote_host_key_error(&std::io::Error::other(message)));
        }
        assert!(!is_remote_host_key_error(&std::io::Error::other(
            "server closed connection"
        )));
    }

    #[test]
    fn remote_auth_error_matches_ssh_auth_denied() {
        let err = std::io::Error::other(
            "remote platform detection failed: user@host: Permission denied (publickey).",
        );

        assert!(is_remote_auth_error(&err));
    }

    #[test]
    fn remote_auth_error_matches_keyboard_interactive_denied() {
        let err = std::io::Error::other(
            "remote server status failed: user@host: Permission denied (keyboard-interactive).",
        );

        assert!(is_remote_auth_error(&err));
    }

    #[test]
    fn remote_auth_error_ignores_non_auth_errors() {
        let err = std::io::Error::other("remote platform detection failed: unsupported platform");

        assert!(!is_remote_auth_error(&err));
    }

    #[test]
    fn ssh_check_command_quotes_remote_target() {
        assert_eq!(ssh_check_command("host name"), "ssh 'host name'");
    }
}
