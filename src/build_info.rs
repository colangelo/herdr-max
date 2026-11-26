//! Build identity helpers.

pub const BASE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn channel() -> &'static str {
    non_empty(option_env!("HERDR_BUILD_CHANNEL")).unwrap_or("stable")
}

pub fn build_id() -> Option<&'static str> {
    non_empty(option_env!("HERDR_BUILD_ID"))
}

/// The channel of a herdr-max build (fork issue 330): the version is plain
/// semver, with no channel suffix and no build number.
const MAX_CHANNEL: &str = "max";

pub fn version() -> String {
    format_version(BASE_VERSION, channel(), build_id())
}

/// What `--version` prints after `herdr `: the version, and for a herdr-max
/// build its build id (commit date and short sha), which is for tracing only
/// and never part of [`version`].
pub fn version_line() -> String {
    format_version_line(BASE_VERSION, channel(), build_id())
}

fn format_version(base: &str, channel: &str, build_id: Option<&str>) -> String {
    match channel {
        "stable" | MAX_CHANNEL => base.to_string(),
        channel => match build_id {
            Some(build_id) => format!("{base}-{channel}.{build_id}"),
            None => format!("{base}-{channel}"),
        },
    }
}

fn format_version_line(base: &str, channel: &str, build_id: Option<&str>) -> String {
    let version = format_version(base, channel, build_id);
    match build_id {
        Some(build_id) if channel == MAX_CHANNEL => format!("{version} (build {build_id})"),
        _ => version,
    }
}

pub fn is_preview() -> bool {
    channel() == "preview"
}

fn non_empty(value: Option<&'static str>) -> Option<&'static str> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn stable_version_defaults_to_cargo_version() {
        assert!(!super::version().is_empty());
    }

    #[test]
    fn a_herdr_max_build_is_plain_semver_with_the_build_id_in_the_version_line_only() {
        assert_eq!(
            super::format_version("0.10.0", "max", Some("20261126-ab12cd3")),
            "0.10.0"
        );
        assert_eq!(
            super::format_version_line("0.10.0", "max", Some("20261126-ab12cd3")),
            "0.10.0 (build 20261126-ab12cd3)"
        );
        assert_eq!(super::format_version_line("0.10.0", "max", None), "0.10.0");
        // The other channels keep their forms.
        assert_eq!(super::format_version("0.9.5", "stable", None), "0.9.5");
        assert_eq!(
            super::format_version("0.9.5", "ac-beta", Some("222-bonucci")),
            "0.9.5-ac-beta.222-bonucci"
        );
        assert_eq!(
            super::format_version_line("0.9.5", "ac-beta", Some("222-bonucci")),
            "0.9.5-ac-beta.222-bonucci"
        );
    }
}
