//! The bus credential file: `NATS_URL`, `NATS_USER` and `NATS_PASSWORD` lines
//! in the format agent-bell reads, plus optional `NATS_JS_DOMAIN`,
//! `TASKS_BUCKET` and `NATS_INBOX_PREFIX`.
//!
//! The server takes the credential only from this file, never from its own
//! environment (a pane or the launching shell could set that) and never from
//! the vault. The file must be a regular file of the current user that nobody
//! else can read. The password never reaches a log or an error message.

use std::fmt;
use std::path::Path;

const DEFAULT_BUCKET: &str = "TASKS";

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct BusEnv {
    pub url: String,
    pub user: String,
    pub password: String,
    /// The JetStream domain (`hub` in the fleet).
    pub js_domain: Option<String>,
    pub bucket: String,
    /// Replaces the default reply inbox prefix when the file names one.
    pub inbox_prefix: Option<String>,
}

impl fmt::Debug for BusEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BusEnv")
            .field("url", &self.url)
            .field("user", &self.user)
            .field("password", &"<hidden>")
            .field("js_domain", &self.js_domain)
            .field("bucket", &self.bucket)
            .field("inbox_prefix", &self.inbox_prefix)
            .finish()
    }
}

/// Read and check the credential file at `path`.
pub(crate) fn load(path: &Path) -> Result<BusEnv, String> {
    let mut file = std::fs::File::open(path).map_err(|err| {
        format!(
            "cannot read the bus credential file {}: {err}",
            path.display()
        )
    })?;
    check_file(&file, path)?;
    let mut text = String::new();
    std::io::Read::read_to_string(&mut file, &mut text).map_err(|err| {
        format!(
            "cannot read the bus credential file {}: {err}",
            path.display()
        )
    })?;
    parse(&text).map_err(|why| format!("bus credential file {}: {why}", path.display()))
}

#[cfg(unix)]
fn check_file(file: &std::fs::File, path: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;

    let meta = file
        .metadata()
        .map_err(|err| format!("cannot inspect {}: {err}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    // SAFETY: geteuid has no preconditions and cannot fail.
    let me = unsafe { libc::geteuid() };
    if meta.uid() != me {
        return Err(format!(
            "{} is not owned by the current user",
            path.display()
        ));
    }
    if meta.mode() & 0o077 != 0 {
        return Err(format!(
            "{} is readable by others (mode {:o}); run chmod 600 on it",
            path.display(),
            meta.mode() & 0o777
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_file(_file: &std::fs::File, path: &Path) -> Result<(), String> {
    Err(format!(
        "{}: bus credentials are only supported on unix hosts",
        path.display()
    ))
}

fn parse(text: &str) -> Result<BusEnv, String> {
    let mut url = None;
    let mut user = None;
    let mut password = None;
    let mut js_domain = None;
    let mut bucket = None;
    let mut inbox_prefix = None;
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("line {} is not KEY=VALUE", index + 1));
        };
        let value = unquote(value.trim());
        let slot = match key.trim() {
            "NATS_URL" => &mut url,
            "NATS_USER" => &mut user,
            "NATS_PASSWORD" => &mut password,
            "NATS_JS_DOMAIN" => &mut js_domain,
            "TASKS_BUCKET" => &mut bucket,
            "NATS_INBOX_PREFIX" => &mut inbox_prefix,
            // Other keys belong to other readers of the same file.
            _ => continue,
        };
        *slot = (!value.is_empty()).then(|| value.to_owned());
    }
    Ok(BusEnv {
        url: url.ok_or("NATS_URL is missing")?,
        user: user.ok_or("NATS_USER is missing")?,
        password: password.ok_or("NATS_PASSWORD is missing")?,
        js_domain,
        bucket: bucket.unwrap_or_else(|| DEFAULT_BUCKET.to_owned()),
        inbox_prefix,
    })
}

fn unquote(value: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_credential_file_is_parsed_with_defaults_and_quotes() {
        let env = parse(
            "# rendered by infra\nNATS_URL=nats://bus:4222\nexport NATS_USER='herdr-m4m'\nNATS_PASSWORD=\"s3cr3t pass\"\nNATS_JS_DOMAIN=hub\nOTHER=ignored\n",
        )
        .unwrap();
        assert_eq!(env.url, "nats://bus:4222");
        assert_eq!(env.user, "herdr-m4m");
        assert_eq!(env.password, "s3cr3t pass");
        assert_eq!(env.js_domain.as_deref(), Some("hub"));
        assert_eq!(env.bucket, "TASKS");
        assert_eq!(env.inbox_prefix, None);

        let env = parse("NATS_URL=x\nNATS_USER=u\nNATS_PASSWORD=p\nTASKS_BUCKET=T2\nNATS_INBOX_PREFIX=_INBOX_mine\n")
            .unwrap();
        assert_eq!(env.bucket, "T2");
        assert_eq!(env.inbox_prefix.as_deref(), Some("_INBOX_mine"));
    }

    #[test]
    fn a_credential_file_must_name_url_user_and_password() {
        for (text, missing) in [
            ("NATS_USER=u\nNATS_PASSWORD=p\n", "NATS_URL"),
            ("NATS_URL=x\nNATS_PASSWORD=p\n", "NATS_USER"),
            ("NATS_URL=x\nNATS_USER=u\nNATS_PASSWORD=\n", "NATS_PASSWORD"),
        ] {
            let err = parse(text).unwrap_err();
            assert!(err.contains(missing), "{err}");
        }
        assert!(parse("not a pair\n").unwrap_err().contains("line 1"));
    }

    #[test]
    fn the_password_never_shows_in_debug_output() {
        let env = parse("NATS_URL=x\nNATS_USER=u\nNATS_PASSWORD=hunter2\n").unwrap();
        assert!(!format!("{env:?}").contains("hunter2"));
    }

    #[cfg(unix)]
    mod files {
        use std::os::unix::fs::PermissionsExt;

        use super::*;

        fn scratch(name: &str) -> std::path::PathBuf {
            let dir = std::env::temp_dir().join(format!(
                "herdr-bus-env-{name}-{}-{}",
                std::process::id(),
                crate::ulid::new_ulid()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        const BODY: &str = "NATS_URL=nats://bus:4222\nNATS_USER=u\nNATS_PASSWORD=hunter2\n";

        #[test]
        fn a_private_regular_file_loads() {
            let dir = scratch("ok");
            let path = dir.join("bus.env");
            std::fs::write(&path, BODY).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert_eq!(load(&path).unwrap().user, "u");
            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn a_file_others_can_read_is_refused_without_leaking_the_password() {
            let dir = scratch("mode");
            let path = dir.join("bus.env");
            std::fs::write(&path, BODY).unwrap();
            for mode in [0o640, 0o604, 0o644, 0o666] {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
                let err = load(&path).unwrap_err();
                assert!(err.contains("readable by others"), "{mode:o}: {err}");
                assert!(!err.contains("hunter2"), "{err}");
            }
            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn a_missing_file_and_a_directory_are_refused() {
            let dir = scratch("missing");
            let err = load(&dir.join("nope.env")).unwrap_err();
            assert!(err.contains("cannot read"), "{err}");
            let err = load(&dir).unwrap_err();
            assert!(!err.is_empty());
            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn a_symlink_to_a_private_file_is_followed_but_its_target_is_judged() {
            let dir = scratch("link");
            let target = dir.join("real.env");
            std::fs::write(&target, BODY).unwrap();
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
            let link = dir.join("bus.env");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert!(load(&link).unwrap_err().contains("readable by others"));
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}
