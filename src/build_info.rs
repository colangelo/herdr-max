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

/// What `--version` prints: `herdr-max 0.11.0 (build ...)` for a herdr-max build (fork issue
/// 331), `herdr <version>` for every other channel. The version is always the second word.
pub fn version_output() -> String {
    format_version_output(BASE_VERSION, channel(), build_id())
}

fn format_version_output(base: &str, channel: &str, build_id: Option<&str>) -> String {
    let name = if channel == MAX_CHANNEL {
        "herdr-max"
    } else {
        "herdr"
    };
    format!("{name} {}", format_version_line(base, channel, build_id))
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

    #[test]
    fn the_herdr_max_build_names_itself_herdr_max_and_the_other_channels_keep_herdr() {
        assert_eq!(
            super::format_version_output("0.11.0", "max", Some("20261126-9d5a725")),
            "herdr-max 0.11.0 (build 20261126-9d5a725)"
        );
        assert_eq!(
            super::format_version_output("0.9.5", "ac-beta", Some("222-bonucci")),
            "herdr 0.9.5-ac-beta.222-bonucci"
        );
        assert_eq!(
            super::format_version_output("0.9.5", "stable", None),
            "herdr 0.9.5"
        );
        assert_eq!(
            super::format_version_output("0.9.5", "preview", Some("x")),
            "herdr 0.9.5-preview.x"
        );
    }
}
