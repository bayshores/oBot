//! openbot-core: everything that runs on the bot's "computer".
//!
//! - `api`      HTTP + WebSocket server the UI talks to
//! - `runner`   runs Claude Code (`claude -p ...`) for each bot and turns its output into chat messages
//! - `mcp`      tiny MCP server that Claude Code calls to ask the user for permission
//! - `db`       SQLite storage for bots, messages and usage
//! - `config`   settings file (token, auth mode, permissions, vault, ...)
//! - `security` device pairing, per-device tokens, Origin/Host checks, failed-login limits

pub mod api;
pub mod computer;
pub mod config;
pub mod db;
pub mod markdown;
pub mod mcp;
pub mod review;
pub mod routines;
pub mod runner;
pub mod security;
pub mod state;
pub mod tools;
pub mod util;

pub use api::{start, ServeOptions, StartedServer};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
