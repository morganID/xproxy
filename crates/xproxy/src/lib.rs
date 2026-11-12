//! xproxy: the GPROXY engine as a library and a command line.
//!
//! GPROXY is a self-hosted gateway for LLM APIs: it holds your upstream
//! accounts — API keys, the OAuth logins behind coding agents, browser cookies
//! — and gives everything else one base URL, one key and model names you
//! choose.
//!
//! ```text
//!   xproxy serve ──────────────────────────────┐
//!                                              │
//!   xproxy provider|account|route|key|login ───┤
//!                                              ▼
//!                                  crate::engine::Gateway
//!                                              │
//!                     gproxy_host_tauri::Desktop ── one App, one snapshot,
//!                                              │      one SQLite file
//!                              data plane on 127.0.0.1:8787
//!                                              │
//!              Claude Code · Codex CLI · anything speaking OpenAI, Claude
//!              or Gemini
//! ```
//!
//! # What this crate is
//!
//! A thin, tested surface over upstream's assembly. Nothing of the engine is
//! forked: `gproxy-ref/` is the upstream repository at a pinned commit, and the
//! workspace manifest points at its crates by path. What is added here is
//!
//! - [`engine::Gateway`] — the engine's lifetime as a value that can be started
//!   and stopped, instead of upstream's `OnceLock`;
//! - [`cli`] — every operation that changes the instance, one command each;
//! - [`catalog`] — what each provider serves, cached so a listing costs no
//!   network;
//! - [`refresh`] — the token check the engine does not schedule for itself.
//!
//! # Using it as a library
//!
//! ```no_run
//! # async fn run() -> Result<(), xproxy::Error> {
//! let gateway = xproxy::Gateway::local()?;
//! let status = gateway.start().await?;
//! println!("{}", status.base_url.unwrap_or_default());
//!
//! let desktop = gateway.desktop().await?;
//! let data = desktop.app().data();
//! let operations = desktop.operations(&data);
//! let users = operations.users().list(Default::default()).await?;
//! println!("{} users", users.total);
//! # Ok(())
//! # }
//! ```

pub mod catalog;
pub mod cli;
pub mod engine;
pub mod error;
pub mod logging;
pub mod refresh;
pub mod util;

pub use engine::{Gateway, Status, data_dir};
pub use error::{Error, Result};

/// Wall clock in milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
