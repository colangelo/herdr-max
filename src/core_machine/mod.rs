//! Core machines (openspec/changes/core-phase5): another Mac or VM added to
//! the primary herdr-core as a machine, reached through herdr's own link
//! command, `herdr core-link`, which is the dial gate (D2, D10). Unix only,
//! as the core backend is.

pub(crate) mod command;
mod dialer;
pub(crate) mod link;
mod name;
pub(crate) mod panes;
pub(crate) mod presence;
mod ssh;
mod state;

pub(crate) use name::MachineName;

#[cfg(test)]
mod tests;
