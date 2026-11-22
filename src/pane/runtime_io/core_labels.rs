//! The facts and writes of `herdr.seen` and `herdr.pinned` label sharing between herdr and
//! herdr-core (openspec/changes/core-phase4, D11): what the label thread
//! reads from core's events and what the server's reducer asks it to write.
//! Pure data.

/// A terminal's address in this server's primary core session (`t7`).
pub(crate) type CoreAddress = String;

/// What core says about one of herdr's terminals.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Observed {
    pub(crate) turn: u64,
    /// `herdr.seen`; a missing or unreadable label is `None`, never `0`.
    pub(crate) seen: Option<u64>,
    /// `herdr.pinned` present and non-empty.
    pub(crate) pinned: bool,
}

/// One fact from core's event stream, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CoreFact {
    /// Every matching terminal, at subscribe or after `missed`.
    Snapshot(Vec<(CoreAddress, Observed)>),
    /// A terminal that newly matches (`created`, or `labels` with a record).
    Upsert {
        address: CoreAddress,
        observed: Observed,
    },
    Turn {
        address: CoreAddress,
        turn: u64,
    },
    Labels {
        address: CoreAddress,
        seen: Option<u64>,
        pinned: bool,
    },
    Gone {
        address: CoreAddress,
    },
    /// The subscription ended; a snapshot starts it again.
    Disconnected,
}

/// A label herdr writes on one terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LabelWrite {
    Seen {
        address: CoreAddress,
        turn: u64,
    },
    /// `herdr.pinned=1`, or `herdr.pinned=` (removes it).
    Pinned {
        address: CoreAddress,
        pinned: bool,
    },
}
