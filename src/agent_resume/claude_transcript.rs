//! The current model, effort and permission mode of a Claude session, read
//! from the end of its transcript (fork issue 123).
//!
//! The Claude hook reports these as a session changes them. A pane that has
//! not fired the hook since herdr was upgraded has no such report, so restore
//! reads the same facts itself, with the hook's rules, from
//! `<claude config dir>/projects/<any project>/<session id>.jsonl`. Only the tail
//! is read, and only for a restore or its preview, never per frame.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// As in the hook: enough for the last turns of a long session.
const TRANSCRIPT_TAIL_BYTES: u64 = 512 * 1024;
const RESUME_MODES: &[&str] = &[
    "acceptEdits",
    "auto",
    "bypassPermissions",
    "default",
    "manual",
    "dontAsk",
    "plan",
];
const RESUME_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

#[derive(Debug, Default, PartialEq, Eq)]
pub struct TranscriptFacts {
    pub mode: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub saw_bypass: bool,
}

/// A value the hook would take: `claude-opus-5-5`, `gpt-6-astra`,
/// `claude-opus-5-5[1m]`; never `<synthetic>`.
fn plain(value: &str) -> bool {
    (1..=200).contains(&value.len())
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'.' | b'_' | b':' | b'/' | b'[' | b']' | b'-')
        })
}

/// When a record was written, in unix ms (`"timestamp": "2026-09-29T11:09:29.659Z"`).
fn record_time_ms(entry: &serde_json::Value) -> Option<i64> {
    let text = entry.get("timestamp")?.as_str()?;
    let at =
        time::OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339).ok()?;
    i64::try_from(at.unix_timestamp_nanos() / 1_000_000).ok()
}

/// The facts of the transcript at `path`, from its last lines.
///
/// With `since_ms` (when the agent process that runs now started), only
/// records written by that process count: a record older than it belongs to an
/// earlier run of the session, which may have been in another mode. A record
/// with no timestamp (`permission-mode`) counts only after a timestamped one
/// from this process.
pub fn transcript_facts(path: &Path, since_ms: Option<i64>) -> TranscriptFacts {
    let mut facts = TranscriptFacts::default();
    let Ok(mut file) = std::fs::File::open(path) else {
        return facts;
    };
    let Ok(len) = file.seek(SeekFrom::End(0)) else {
        return facts;
    };
    let start = len.saturating_sub(TRANSCRIPT_TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return facts;
    }
    let mut tail = Vec::new();
    if file.read_to_end(&mut tail).is_err() {
        return facts;
    }
    let mut lines = tail.split(|byte| *byte == b'\n');
    if start > 0 {
        // Cut mid-line: the first piece is not a whole entry.
        lines.next();
    }
    let mut live = since_ms.is_none();
    for line in lines {
        let Ok(entry) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        if entry
            .get("isSidechain")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        {
            continue;
        }
        if let (Some(since), Some(written)) = (since_ms, record_time_ms(&entry)) {
            live = written >= since;
        }
        if !live {
            continue;
        }
        let mode = || {
            entry
                .get("permissionMode")
                .and_then(serde_json::Value::as_str)
                .filter(|mode| RESUME_MODES.contains(mode))
        };
        match entry.get("type").and_then(serde_json::Value::as_str) {
            Some("user" | "permission-mode") => {
                if let Some(mode) = mode() {
                    facts.saw_bypass |= mode == "bypassPermissions";
                    facts.mode = Some(mode.to_string());
                }
            }
            Some("assistant") => {
                if let Some(model) = entry
                    .pointer("/message/model")
                    .and_then(serde_json::Value::as_str)
                    .filter(|model| plain(model))
                {
                    facts.model = Some(model.to_string());
                }
                if let Some(effort) = entry
                    .get("effort")
                    .and_then(serde_json::Value::as_str)
                    .filter(|effort| RESUME_EFFORTS.contains(effort))
                {
                    facts.effort = Some(effort.to_string());
                }
            }
            _ => {}
        }
    }
    facts
}

/// The resume command the hook would report from these facts alone.
pub fn resume_argv_from_facts(session_id: &str, facts: &TranscriptFacts) -> Vec<String> {
    let mut argv = vec![
        "claude".to_string(),
        "--resume".to_string(),
        session_id.to_string(),
    ];
    if let Some(model) = &facts.model {
        argv.extend(["--model".to_string(), model.clone()]);
    }
    if let Some(effort) = &facts.effort {
        argv.extend(["--effort".to_string(), effort.clone()]);
    }
    if facts.saw_bypass {
        argv.push("--allow-dangerously-skip-permissions".to_string());
    }
    if let Some(mode) = &facts.mode {
        argv.extend(["--permission-mode".to_string(), mode.clone()]);
    }
    argv
}

/// The transcript of `session_id` under `config_dir`. A session can sit in
/// more than one project folder (a moved repo, a resume from another
/// directory); the most recently written copy is the live one.
pub fn find_transcript(config_dir: &Path, session_id: &str) -> Option<PathBuf> {
    if session_id.is_empty() || session_id.contains(['/', '\\']) || session_id.contains("..") {
        return None;
    }
    let file = format!("{session_id}.jsonl");
    std::fs::read_dir(config_dir.join("projects"))
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join(&file))
        .filter_map(|path| {
            let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
            Some((modified, path))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

/// `CLAUDE_CONFIG_DIR`, else `~/.claude`, as Claude Code itself resolves it.
fn claude_config_dir() -> Option<PathBuf> {
    crate::config::path_env("CLAUDE_CONFIG_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::config::home_env().map(|home| PathBuf::from(home).join(".claude")))
}

/// The resume command of a Claude session with no hook report, from its
/// transcript. `None` when the transcript cannot be found.
///
/// `since_ms` is when the agent process that ran in the pane started (see
/// `transcript_facts`); `None` counts every record.
pub fn claude_transcript_resume(session_id: &str, since_ms: Option<i64>) -> Option<Vec<String>> {
    let path = find_transcript(&claude_config_dir()?, session_id)?;
    Some(resume_argv_from_facts(
        session_id,
        &transcript_facts(&path, since_ms),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "herdr-claude-transcript-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write_lines(path: &Path, lines: &[serde_json::Value]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let body: String = lines.iter().map(|line| format!("{line}\n")).collect();
        std::fs::write(path, body).unwrap();
    }

    fn user(mode: &str) -> serde_json::Value {
        serde_json::json!({"type": "user", "permissionMode": mode, "message": {"role": "user"}})
    }

    fn assistant(model: &str, effort: &str) -> serde_json::Value {
        serde_json::json!({"type": "assistant", "effort": effort, "message": {"model": model}})
    }

    #[test]
    fn the_last_turn_decides_model_effort_and_mode() {
        let scratch = Scratch::new("facts");
        let path = scratch.0.join("t.jsonl");
        write_lines(
            &path,
            &[
                user("bypassPermissions"),
                assistant("claude-opus-5-5", "high"),
                user("auto"),
                assistant("claude-sonnet-5-5", "low"),
                assistant("<synthetic>", "banana"),
                serde_json::json!({"type": "assistant", "isSidechain": true,
                    "effort": "max", "message": {"model": "claude-haiku-4-5"}}),
            ],
        );

        assert_eq!(
            resume_argv_from_facts("s1", &transcript_facts(&path, None)),
            [
                "claude",
                "--resume",
                "s1",
                "--model",
                "claude-sonnet-5-5",
                "--effort",
                "low",
                "--allow-dangerously-skip-permissions",
                "--permission-mode",
                "auto",
            ]
        );
    }

    #[test]
    fn a_long_transcript_is_read_from_its_tail() {
        let scratch = Scratch::new("tail");
        let path = scratch.0.join("t.jsonl");
        let filler = serde_json::json!({"type": "progress", "pad": "x".repeat(1000)});
        let mut lines = vec![assistant("claude-opus-5-5", "high")];
        lines.extend(std::iter::repeat_n(filler, 700));
        lines.push(assistant("gpt-6-astra", "medium"));
        write_lines(&path, &lines);

        let facts = transcript_facts(&path, None);
        assert_eq!(facts.model.as_deref(), Some("gpt-6-astra"));
        assert_eq!(facts.effort.as_deref(), Some("medium"));
    }

    fn at(ts: &str, mut line: serde_json::Value) -> serde_json::Value {
        line["timestamp"] = serde_json::json!(ts);
        line
    }

    fn mode_record(mode: &str) -> serde_json::Value {
        serde_json::json!({"type": "permission-mode", "permissionMode": mode})
    }

    /// 2026-09-29T13:40:00Z, when the running process started.
    const STARTED_MS: i64 = 1_790_689_200_000;

    #[test]
    fn records_from_before_the_running_process_are_ignored() {
        // wP:p9: a plain restore at 11:09 wrote "default"; the process that
        // runs now was started at 13:40 in bypass and wrote nothing since.
        let scratch = Scratch::new("stale");
        let path = scratch.0.join("t.jsonl");
        write_lines(
            &path,
            &[
                at(
                    "2026-09-29T05:00:17.877Z",
                    assistant("claude-opus-5-5", "medium"),
                ),
                at("2026-09-29T11:09:29.659Z", user("default")),
                mode_record("default"),
            ],
        );

        let facts = transcript_facts(&path, Some(STARTED_MS));

        assert_eq!(
            facts,
            TranscriptFacts::default(),
            "nothing is newer than the process"
        );
    }

    #[test]
    fn an_untimed_mode_record_counts_after_a_newer_timed_one() {
        let scratch = Scratch::new("untimed");
        let path = scratch.0.join("t.jsonl");
        write_lines(
            &path,
            &[
                mode_record("plan"),
                at("2026-09-29T11:00:00.000Z", user("default")),
                at(
                    "2026-09-29T14:00:00.000Z",
                    assistant("claude-sonnet-5-5", "low"),
                ),
                mode_record("acceptEdits"),
            ],
        );

        let facts = transcript_facts(&path, Some(STARTED_MS));

        assert_eq!(facts.mode.as_deref(), Some("acceptEdits"));
        assert_eq!(facts.model.as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(facts.effort.as_deref(), Some("low"));
    }

    #[test]
    fn without_a_process_start_every_record_counts() {
        let scratch = Scratch::new("nostart");
        let path = scratch.0.join("t.jsonl");
        write_lines(
            &path,
            &[
                at("2026-09-29T11:09:29.659Z", user("default")),
                mode_record("plan"),
            ],
        );

        assert_eq!(transcript_facts(&path, None).mode.as_deref(), Some("plan"));
    }

    #[test]
    fn of_several_copies_the_newest_transcript_wins() {
        // w0:p1: the repo moved, and the session file sits in two other
        // project folders, none of them the pane's cwd.
        let scratch = Scratch::new("copies");
        let old = scratch.0.join("projects/-a/s1.jsonl");
        let new = scratch.0.join("projects/-b/s1.jsonl");
        write_lines(&old, &[user("auto")]);
        write_lines(&new, &[user("auto")]);
        let past = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(past)
            .unwrap();

        assert_eq!(find_transcript(&scratch.0, "s1"), Some(new));
    }

    #[test]
    fn a_missing_transcript_gives_no_facts() {
        assert_eq!(
            transcript_facts(Path::new("/nonexistent/herdr/t.jsonl"), None),
            TranscriptFacts::default()
        );
    }

    #[test]
    fn the_transcript_is_found_in_its_project_folder_or_any_other() {
        let scratch = Scratch::new("find");
        let direct = scratch
            .0
            .join("projects/-Users-ac--sync-dev-herdr/s1.jsonl");
        write_lines(&direct, &[user("auto")]);
        let moved = scratch.0.join("projects/-elsewhere/s2.jsonl");
        write_lines(&moved, &[user("auto")]);

        assert_eq!(find_transcript(&scratch.0, "s1"), Some(direct));
        assert_eq!(find_transcript(&scratch.0, "s2"), Some(moved));
        assert_eq!(find_transcript(&scratch.0, "s3"), None);
        assert_eq!(find_transcript(&scratch.0, "../s1"), None);
    }
}
