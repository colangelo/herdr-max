//! Shared wire protocol and presentation encoding code.

pub mod endpoint;
mod resource_facts;
pub use resource_facts::*;
pub(crate) mod render_ansi;
pub(crate) mod surface_delta;
pub(crate) mod surface_reuse;
pub(crate) mod surface_scroll;
mod wire;

pub use wire::*;
