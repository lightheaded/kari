//! kari core: reads Claude Code and herdr state, derives board state, stores cards.

pub mod account;
pub mod agents;
pub mod api;
pub mod client;
pub mod engine;
pub mod estimate;
pub mod herdr;
pub mod hooks;
pub mod hub;
pub mod infer;
pub mod keychain;
pub mod launcher;
pub mod model;
pub mod net;
pub mod paths;
pub mod planner;
pub mod quota;
pub mod registry;
pub mod statusline;
pub mod store;
pub mod summary;
pub mod transcript;
pub mod tunnel;

pub use engine::{Engine, Event};
pub use model::*;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
