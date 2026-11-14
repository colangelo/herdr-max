//! The herdr-core backend behind `PaneIo` (openspec/changes/core-backend).
//! Unix only: core is reached over a Unix socket.

// Pieces land one task at a time; the backend wires them in task 4.1.
#[allow(dead_code)]
pub(crate) mod frame;
