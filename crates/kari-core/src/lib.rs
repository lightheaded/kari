//! kari core: reads Claude Code and herdr state, derives board state, stores cards.

pub mod account;
pub mod agents;
pub mod api;
pub mod attach;
pub mod client;
pub mod engine;
pub mod estimate;
pub mod herdr;
pub mod hooks;
pub mod hub;
pub mod hubapi;
pub mod infer;
pub mod keychain;
pub mod launcher;
pub mod link;
pub mod model;
pub mod net;
pub mod outbox;
pub mod owner;
pub mod paths;
pub mod peer;
pub mod planner;
pub mod proc;
pub mod quota;
pub mod registry;
pub mod remote;
pub mod server;
pub mod split;
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
