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
    /// A `working` herdr offered that a later record replaced before it was
    /// written. Core's turn rises only when its record leaves `working`, so it
    /// goes out just before the next record (openspec/changes/core-phase4, D6).
    owed_working: Option<CoreStatusRecord>,
    not_before: Option<Instant>,
}

/// What one write puts in the program's terminal: the record, after the
/// `working` that was owed, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MirrorWrite {
    owed_working: Option<CoreStatusRecord>,
    record: CoreStatusRecord,
}

impl MirrorWrite {
    pub(super) fn encode(&self) -> Vec<u8> {
        let mut bytes = self
            .owed_working
            .as_ref()
            .map(CoreStatusRecord::encode)
            .unwrap_or_default();
        bytes.extend_from_slice(&self.record.encode());
        bytes
    }
}

impl StatusMirror {
    /// Every chunk core sends for the pane, before it is parsed.
    pub(super) fn observe(&mut self, bytes: &[u8], now: Instant) {
        self.scanner.feed(bytes);
        self.last_output = Some(now);
    }

    /// A new record replaces any waiting one. A record already written, or a
    /// `clear` when nothing was ever written, needs no write, unless a
    /// `working` is owed: then it carries that turn's end to core.
    pub(super) fn offer(&mut self, record: CoreStatusRecord) {
        let working = record.is_working();
        if working
            && !self
                .written
                .as_ref()
                .is_some_and(CoreStatusRecord::is_working)
        {
            self.owed_working = Some(record.clone());
        }
        let redundant = match &self.written {
            Some(written) => *written == record,
            None => record.is_clear(),
        };
        let carries_turn = !working && self.owed_working.is_some();
        self.pending = (!redundant || carries_turn).then_some(record);
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

    /// What to write now, if the gate is open.
    pub(super) fn take_due(&mut self, now: Instant) -> Option<MirrorWrite> {
        if self.wake_in(now)? != Duration::ZERO {
            return None;
        }
        let record = self.pending.take()?;
        let owed_working = (!record.is_working())
            .then(|| self.owed_working.clone())
            .flatten();
        Some(MirrorWrite {
            owed_working,
            record,
        })
    }

    pub(super) fn written(&mut self, write: MirrorWrite) {
        self.written = Some(write.record);
        self.owed_working = None;
        self.not_before = None;
    }

    /// The terminal's buffer was full: try again later unless a newer record
    /// arrived meanwhile. An owed `working` stays owed.
    pub(super) fn busy(&mut self, write: MirrorWrite, now: Instant) {
        self.pending.get_or_insert(write.record);
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
        assert_eq!(mirror.take_due(later + QUIET), Some(plain(working())));
        assert_eq!(mirror.wake_in(later + QUIET), None, "nothing left");
    }

    #[test]
    fn with_no_output_yet_a_record_is_due_at_once() {
        let now = Instant::now();
        let mut mirror = StatusMirror::default();
        mirror.offer(working());
        assert_eq!(mirror.take_due(now), Some(plain(working())));
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
        assert_eq!(due, plain(working()));
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
            Some(plain(CoreStatusRecord::Clear))
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
        assert_eq!(retried, plain(working()));
        mirror.written(retried);

        let later = now + BUSY_RETRY;
        mirror.offer(idle());
        let due = mirror.take_due(later).expect("due");
        mirror.offer(CoreStatusRecord::Clear);
        mirror.busy(due, later);
        assert_eq!(
            mirror.take_due(later + BUSY_RETRY),
            Some(plain(CoreStatusRecord::Clear)),
            "the newer record, not the one that met the full buffer"
        );
    }

    fn done() -> CoreStatusRecord {
        CoreStatusRecord::Report {
            word: CoreStatusWord::Done,
            app: CoreStatusApp::parse("claude"),
        }
    }

    fn plain(record: CoreStatusRecord) -> MirrorWrite {
        MirrorWrite {
            owed_working: None,
            record,
        }
    }

    fn after_working(record: CoreStatusRecord) -> MirrorWrite {
        MirrorWrite {
            owed_working: Some(working()),
            record,
        }
    }

    /// Writes what is due once the pane has been quiet, as the parser thread does.
    fn write_due(mirror: &mut StatusMirror, at: Instant) -> Option<MirrorWrite> {
        let due = mirror.take_due(at + QUIET)?;
        mirror.written(due.clone());
        Some(due)
    }

    // core-phase4 verifier 1: a `working` replaced before its write goes out
    // in front of the record that replaced it, so core's turn still rises.
    #[test]
    fn a_coalesced_working_is_written_before_the_record_that_replaced_it() {
        let now = Instant::now();
        let mut mirror = StatusMirror::default();
        mirror.offer(idle());
        write_due(&mut mirror, now);

        mirror.observe(b"output", now);
        mirror.offer(working());
        mirror.offer(done());
        let due = write_due(&mut mirror, now).expect("due");
        assert_eq!(due, after_working(done()));
        assert_eq!(
            due.encode(),
            [working().encode(), done().encode()].concat(),
            "one write, working first"
        );
        assert!(due.encode().len() < 256);

        mirror.offer(working());
        mirror.offer(done());
        assert_eq!(
            write_due(&mut mirror, now),
            Some(after_working(done())),
            "a second unread turn: done again, still a new turn"
        );
        assert_eq!(write_due(&mut mirror, now), None, "the debt is paid");
    }

    // core-phase4 verifier 2: no `working` is invented, and a written one
    // settles the debt.
    #[test]
    fn no_working_is_owed_without_one_offered_or_after_one_was_written() {
        let now = Instant::now();
        let mut mirror = StatusMirror::default();
        mirror.offer(idle());
        write_due(&mut mirror, now);
        mirror.offer(done());
        assert_eq!(write_due(&mut mirror, now), Some(plain(done())));

        mirror.offer(working());
        assert_eq!(write_due(&mut mirror, now), Some(plain(working())));
        mirror.offer(done());
        assert_eq!(
            write_due(&mut mirror, now),
            Some(plain(done())),
            "core saw working itself"
        );
    }

    #[test]
    fn a_full_buffer_keeps_the_owed_working() {
        let now = Instant::now();
        let mut mirror = StatusMirror::default();
        mirror.offer(idle());
        write_due(&mut mirror, now);
        mirror.offer(working());
        mirror.offer(idle());
        let due = mirror.take_due(now + QUIET).expect("due");
        mirror.busy(due, now);
        assert_eq!(
            mirror.take_due(now + BUSY_RETRY + QUIET),
            Some(after_working(idle()))
        );
    }

    #[test]
    fn the_longest_pair_fits_one_write() {
        let long = CoreStatusApp::parse(&"a".repeat(64));
        let write = MirrorWrite {
            owed_working: Some(CoreStatusRecord::Report {
                word: CoreStatusWord::Working,
                app: long.clone(),
            }),
            record: CoreStatusRecord::Report {
                word: CoreStatusWord::Blocked,
                app: long,
            },
        };
        assert!(write.encode().len() < 256, "{}", write.encode().len());
    }
}
