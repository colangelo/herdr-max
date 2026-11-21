//! When a core pane may write its mirrored OSC 7501 record
//! (openspec/changes/core-status-mirror, D3). The pane's parser thread owns one
//! `StatusMirror`: it sees every byte core sends for the pane, so it knows
//! whether the program is inside an escape sequence and how long it has been
//! quiet. Pure: time comes in as arguments.

use std::time::{Duration, Instant};

use crate::pane::runtime_io::CoreStatusRecord;

/// How long the pane's output must be quiet before a record is written.
pub(super) const QUIET: Duration = Duration::from_millis(100);
/// How long to wait after the terminal's buffer was full.
const BUSY_RETRY: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StringKind {
    /// OSC: ends at BEL or ST.
    Osc,
    /// DCS, SOS, PM, APC: end at ST only.
    Other,
}

/// Where a VT parser reading the same bytes would be.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ScanState {
    #[default]
    Ground,
    Escape,
    EscapeIntermediate,
    Csi,
    String(StringKind),
    StringEscape(StringKind),
    /// Inside a UTF-8 character, with this many continuation bytes to come.
    Utf8(u8),
}

fn utf8_lead(byte: u8) -> ScanState {
    match byte {
        0xc2..=0xdf => ScanState::Utf8(1),
        0xe0..=0xef => ScanState::Utf8(2),
        0xf0..=0xf4 => ScanState::Utf8(3),
        _ => ScanState::Ground,
    }
}

fn step(state: ScanState, byte: u8) -> ScanState {
    use ScanState::*;
    match (state, byte) {
        // CAN and SUB cancel any sequence.
        (_, 0x18 | 0x1a) => Ground,
        (String(kind), 0x1b) => StringEscape(kind),
        (String(StringKind::Osc), 0x07) => Ground,
        (String(kind), _) => String(kind),
        (StringEscape(_), b'\\') => Ground,
        // An ESC inside a string ends it and starts a new sequence.
        (StringEscape(_), byte) => step(Escape, byte),
        (_, 0x1b) => Escape,
        (Ground, byte) => utf8_lead(byte),
        (Utf8(left), 0x80..=0xbf) => match left {
            0 | 1 => Ground,
            left => Utf8(left - 1),
        },
        (Utf8(_), byte) => utf8_lead(byte),
        (Escape, b'[') => Csi,
        (Escape, b']') => String(StringKind::Osc),
        (Escape, b'P' | b'X' | b'^' | b'_') => String(StringKind::Other),
        (Escape, 0x20..=0x2f) => EscapeIntermediate,
        (Escape | EscapeIntermediate, 0x30..=0x7e) => Ground,
        // C0 controls run without ending the sequence; DEL is ignored.
        (Escape | EscapeIntermediate, _) => state,
        (Csi, 0x40..=0x7e) => Ground,
        (Csi, _) => Csi,
    }
}

/// Tracks whether a byte stream is between escape sequences and characters.
#[derive(Debug, Default)]
pub(super) struct EscapeScanner {
    state: ScanState,
}

impl EscapeScanner {
    /// O(n), no allocation. In ground it skips plain ASCII to the next byte
    /// that can start a sequence or a multi-byte character.
    pub(super) fn feed(&mut self, bytes: &[u8]) {
        let mut rest = bytes;
        while !rest.is_empty() {
            if self.state == ScanState::Ground {
                match rest
                    .iter()
                    .position(|byte| matches!(byte, 0x18 | 0x1a | 0x1b | 0x80..=0xff))
                {
                    Some(start) => rest = &rest[start..],
                    None => return,
                }
            }
            self.state = step(self.state, rest[0]);
            rest = &rest[1..];
        }
    }

    pub(super) fn in_ground(&self) -> bool {
        self.state == ScanState::Ground
    }
}

/// One pane's mirror: the scanner, the quiet timer, the latest record waiting
/// to be written and the last one written.
#[derive(Debug, Default)]
pub(super) struct StatusMirror {
    scanner: EscapeScanner,
    last_output: Option<Instant>,
    pending: Option<CoreStatusRecord>,
    written: Option<CoreStatusRecord>,
    not_before: Option<Instant>,
}

impl StatusMirror {
    /// Every chunk core sends for the pane, before it is parsed.
    pub(super) fn observe(&mut self, bytes: &[u8], now: Instant) {
        self.scanner.feed(bytes);
        self.last_output = Some(now);
    }

    /// A new record replaces any waiting one. A record already written, or a
    /// `clear` when nothing was ever written, needs no write.
    pub(super) fn offer(&mut self, record: CoreStatusRecord) {
        let redundant = match &self.written {
            Some(written) => *written == record,
            None => record.is_clear(),
        };
        self.pending = (!redundant).then_some(record);
    }

    /// How long until a waiting record may be written; `None` when nothing
    /// waits, or the stream is inside a sequence (only more output can end it).
    pub(super) fn wake_in(&self, now: Instant) -> Option<Duration> {
        self.pending.as_ref()?;
        if !self.scanner.in_ground() {
            return None;
        }
        let quiet_at = self.last_output.map_or(now, |last| last + QUIET);
        let at = self.not_before.map_or(quiet_at, |busy| busy.max(quiet_at));
        Some(at.saturating_duration_since(now))
    }

    /// The record to write now, if the gate is open.
    pub(super) fn take_due(&mut self, now: Instant) -> Option<CoreStatusRecord> {
        (self.wake_in(now)? == Duration::ZERO).then(|| self.pending.take())?
    }

    pub(super) fn written(&mut self, record: CoreStatusRecord) {
        self.written = Some(record);
        self.not_before = None;
    }

    /// The terminal's buffer was full: try again later unless a newer record
    /// arrived meanwhile.
    pub(super) fn busy(&mut self, record: CoreStatusRecord, now: Instant) {
        self.pending.get_or_insert(record);
        self.not_before = Some(now + BUSY_RETRY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane::runtime_io::{CoreStatusApp, CoreStatusWord};

    fn ground_after(bytes: &[u8]) -> bool {
        let mut scanner = EscapeScanner::default();
        scanner.feed(bytes);
        scanner.in_ground()
    }

    const WHOLE: &[&[u8]] = &[
        b"plain text\r\n",
        b"\x1b[38;5;196mred\x1b[0m",
        b"\x1b[?1049h",
        b"\x1b]0;title\x07",
        b"\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\",
        b"\x1b]7501;state=working:app=claude\x1b\\",
        b"\x1bP+q544e\x1b\\",
        b"\x1b_Gf=100;AAAA\x1b\\",
        b"\x1b(B",
        b"\x1b7\x1b8",
        "caf\u{e9} \u{2014} \u{1f600}".as_bytes(),
    ];

    #[test]
    fn whole_sequences_end_in_ground() {
        for bytes in WHOLE {
            assert!(ground_after(bytes), "{bytes:?}");
        }
        assert!(ground_after(b"\x1b]0;cut\x18"), "CAN cancels an OSC");
        assert!(ground_after(b"\x1b[12\x1a"), "SUB cancels a CSI");
        assert!(
            ground_after(b"\x1b]0;a\x1b[0m"),
            "ESC in an OSC starts a new sequence"
        );
        assert!(!ground_after(b"\x1bP1$r\x07"), "BEL does not end a DCS");
    }

    /// Each item is one sequence or one character, with no terminator inside.
    const SINGLE: &[&[u8]] = &[
        b"\x1b[38;5;196m",
        b"\x1b[?1049h",
        b"\x1b]0;title\x07",
        b"\x1b]7501;state=working:app=claude\x1b\\",
        b"\x1bP+q544e\x1b\\",
        b"\x1b_Gf=100;AAAA\x1b\\",
        b"\x1b(B",
        b"\x1b7",
        "\u{e9}".as_bytes(),
        "\u{2014}".as_bytes(),
        "\u{1f600}".as_bytes(),
    ];

    #[test]
    fn every_split_inside_a_sequence_or_character_is_not_ground() {
        for bytes in SINGLE {
            for cut in 1..bytes.len() {
                let mut scanner = EscapeScanner::default();
                scanner.feed(&bytes[..cut]);
                assert!(!scanner.in_ground(), "{bytes:?} cut after {cut} bytes");
                scanner.feed(&bytes[cut..]);
                assert!(scanner.in_ground(), "{bytes:?} rejoined at {cut}");
            }
        }
        for bytes in WHOLE {
            for cut in 0..=bytes.len() {
                let mut scanner = EscapeScanner::default();
                scanner.feed(&bytes[..cut]);
                scanner.feed(&bytes[cut..]);
                assert!(scanner.in_ground(), "{bytes:?} split at {cut}");
            }
        }
    }

    fn working() -> CoreStatusRecord {
        CoreStatusRecord::Report {
            word: CoreStatusWord::Working,
            app: CoreStatusApp::parse("claude"),
        }
    }

    fn idle() -> CoreStatusRecord {
        CoreStatusRecord::Report {
            word: CoreStatusWord::Idle,
            app: CoreStatusApp::parse("claude"),
        }
    }

    #[test]
    fn a_record_waits_for_quiet_output_outside_any_sequence() {
        let start = Instant::now();
        let mut mirror = StatusMirror::default();
        mirror.observe(b"thinking\x1b]0;tit", start);
        mirror.offer(working());
        assert_eq!(
            mirror.wake_in(start),
            None,
            "inside an OSC: wait for output"
        );
        assert_eq!(mirror.take_due(start + QUIET * 5), None);

        let later = start + Duration::from_millis(30);
        mirror.observe(b"le\x07", later);
        assert_eq!(mirror.wake_in(later), Some(QUIET));
        assert_eq!(mirror.take_due(later + QUIET / 2), None, "not quiet yet");
        assert_eq!(mirror.take_due(later + QUIET), Some(working()));
        assert_eq!(mirror.wake_in(later + QUIET), None, "nothing left");
    }

    #[test]
    fn with_no_output_yet_a_record_is_due_at_once() {
        let now = Instant::now();
        let mut mirror = StatusMirror::default();
        mirror.offer(working());
        assert_eq!(mirror.take_due(now), Some(working()));
    }

    #[test]
    fn the_latest_record_wins_and_redundant_ones_are_dropped() {
        let now = Instant::now();
        let mut mirror = StatusMirror::default();
        mirror.offer(CoreStatusRecord::Clear);
        assert_eq!(mirror.wake_in(now), None, "nothing was written to clear");

        mirror.observe(b"\x1b[", now);
        mirror.offer(working());
        mirror.offer(idle());
        mirror.offer(working());
        mirror.observe(b"m", now);
        let due = mirror.take_due(now + QUIET).expect("due");
        assert_eq!(due, working());
        mirror.written(due);
        assert_eq!(mirror.take_due(now + QUIET * 2), None, "one write");

        mirror.offer(working());
        assert_eq!(mirror.wake_in(now), None, "already written");
        mirror.offer(idle());
        mirror.offer(working());
        assert_eq!(mirror.wake_in(now), None, "back to what was written");
        mirror.offer(CoreStatusRecord::Clear);
        assert_eq!(
            mirror.take_due(now + QUIET * 2),
            Some(CoreStatusRecord::Clear)
        );
    }

    #[test]
    fn a_full_buffer_retries_later_and_keeps_a_newer_record() {
        let now = Instant::now();
        let mut mirror = StatusMirror::default();
        mirror.offer(working());
        let due = mirror.take_due(now).expect("due");
        mirror.busy(due, now);
        assert_eq!(mirror.wake_in(now), Some(BUSY_RETRY));
        let retried = mirror.take_due(now + BUSY_RETRY).expect("retried");
        assert_eq!(retried, working());
        mirror.written(retried);

        let later = now + BUSY_RETRY;
        mirror.offer(idle());
        let due = mirror.take_due(later).expect("due");
        mirror.offer(CoreStatusRecord::Clear);
        mirror.busy(due, later);
        assert_eq!(
            mirror.take_due(later + BUSY_RETRY),
            Some(CoreStatusRecord::Clear),
            "the newer record, not the one that met the full buffer"
        );
    }
}
