//! The gateway process: assemble the engine, bind the data plane, stop it.
//!
//! # Why this is not `gproxy_host_tauri::engine`
//!
//! Upstream's own engine module is a `OnceLock` — one instance per process,
//! started by whoever asks first and never un-started. That is right for a
//! desktop window whose close ends the process, and wrong for anything that
//! wants to stop serving without exiting: a supervisor, a test, a `serve`
//! command that should answer SIGINT. So this module owns the same value in a
//! `Mutex<Option<Desktop>>` instead; the assembly below it is upstream's,
//! unchanged, and only the lifetime is ours.
//!
//! [`Desktop`] is cheap to clone (an `Arc` inside), so a caller that needs one
//! takes a clone and lets the lock go before it awaits anything.
//!
//! # What starting costs
//!
//! Everything `Desktop::start` already does, in this order: the master key out
//! of the login keychain, `gproxy.toml`, the SQLite schema, the first
//! administrator on a new instance, the gateway key, and the data plane bound
//! on loopback. There is no wizard on purpose — a local instance needs no
//! password, because nobody signs in to it.

use std::path::{Path, PathBuf};

use gproxy_host_tauri::{Desktop, StartError, config, secrets::Keychain};
use serde::Serialize;
use tokio::sync::Mutex;

use crate::error::Error;

/// The bundle identifier, and therefore the name of the data folder.
///
/// `~/Library/Application Support/<identifier>`, which is where a Mac user
/// looks for an application's files and where Tauri's own resolver would put
/// them. `XPROXY_DATA_DIR` overrides it, which is what makes a second instance
/// on the same machine possible.
pub const IDENTIFIER: &str = "dev.xproxy";

/// Where this instance keeps its database.
pub fn data_dir() -> Result<PathBuf, Error> {
    if let Some(path) = std::env::var_os("XPROXY_DATA_DIR") {
        return Ok(PathBuf::from(path));
    }
    std::env::var_os("HOME")
        .map(|home| {
            PathBuf::from(home)
                .join("Library/Application Support")
                .join(IDENTIFIER)
        })
        .ok_or_else(|| Error::new("data_dir", "there is no home directory"))
}

/// What a caller can say about the instance without holding it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// Whether the data plane is bound and answering.
    pub running: bool,
    /// What a client puts in front of `/v1/messages`.
    pub base_url: Option<String>,
    pub port: Option<u16>,
    /// Where the database and `gproxy.toml` live.
    pub data_dir: String,
    /// Whether upstream credentials are sealed by the login keychain.
    pub secrets_sealed: bool,
    /// Identity snapshot revision, which moves on every write.
    pub revision: i64,
    pub started_at_ms: Option<i64>,
    /// The last failure, kept until a start succeeds.
    pub error: Option<String>,
}

/// One running instance, or the reason there is not one.
pub struct Gateway {
    data_dir: PathBuf,
    desktop: Mutex<Option<Desktop>>,
    failure: Mutex<Option<String>>,
    started_at_ms: Mutex<Option<i64>>,
}

impl Gateway {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            desktop: Mutex::new(None),
            failure: Mutex::new(None),
            started_at_ms: Mutex::new(None),
        }
    }

    /// The default instance, at the default path.
    pub fn local() -> Result<Self, Error> {
        Ok(Self::new(data_dir()?))
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// The instance, if one is running.
    pub async fn desktop(&self) -> Result<Desktop, Error> {
        self.desktop
            .lock()
            .await
            .clone()
            .ok_or_else(|| Error::new("not_running", "the proxy is not running"))
    }

    pub async fn status(&self) -> Status {
        let desktop = self.desktop.lock().await.clone();
        let error = self.failure.lock().await.clone();
        let started_at_ms = *self.started_at_ms.lock().await;
        match desktop {
            Some(desktop) => {
                let plane = desktop.data_plane();
                Status {
                    running: true,
                    base_url: Some(plane.base_url.clone()),
                    port: Some(plane.address.port()),
                    data_dir: self.data_dir.display().to_string(),
                    secrets_sealed: desktop.secrets().secrets_are_sealed(),
                    revision: desktop.app().snapshot().revision(),
                    started_at_ms,
                    error,
                }
            }
            None => Status {
                running: false,
                base_url: None,
                port: None,
                data_dir: self.data_dir.display().to_string(),
                // Nothing is sealed because nothing is open.
                secrets_sealed: false,
                revision: 0,
                started_at_ms: None,
                error,
            },
        }
    }

    /// Assemble, bind and serve. A no-op when one is already running.
    pub async fn start(&self) -> Result<Status, Error> {
        let mut slot = self.desktop.lock().await;
        if slot.is_some() {
            drop(slot);
            return Ok(self.status().await);
        }
        tracing::info!(data_dir = %self.data_dir.display(), "starting the engine");
        match Desktop::start(self.data_dir.clone(), &Keychain).await {
            Ok(desktop) => {
                tracing::info!(
                    base_url = %desktop.data_plane().base_url,
                    "the data plane is listening"
                );
                enable_request_log(&desktop).await;
                *slot = Some(desktop);
                *self.started_at_ms.lock().await = Some(crate::now_ms());
                *self.failure.lock().await = None;
                drop(slot);
                Ok(self.status().await)
            }
            Err(error) => {
                // The full text goes to the log and to the caller: a start
                // failure is about this machine — a port taken, a keychain
                // entry gone — and the person is the one who can fix it.
                tracing::error!(%error, "the engine could not start");
                let message = describe(&error);
                *self.failure.lock().await = Some(message.clone());
                Err(Error::new("start_failed", message))
            }
        }
    }

    /// Close the socket and stop the background sync.
    pub async fn stop(&self) -> Result<Status, Error> {
        let mut slot = self.desktop.lock().await;
        if let Some(desktop) = slot.take() {
            tracing::info!("stopping the engine");
            desktop.shutdown();
        }
        *self.started_at_ms.lock().await = None;
        drop(slot);
        Ok(self.status().await)
    }
}

/// Turn on the request log `xproxy log` reads.
///
/// Upstream ships with both switches off, and that is the right default for a
/// server: every logged exchange is two more rows per request, a cost a
/// gateway should not pay unasked. A local instance is a different bargain —
/// the CLI's whole job is to answer "did it work, and what did the upstream
/// say" — so the switches are turned on once, here, and only when both are
/// off. An operator who turns them off deliberately keeps them off.
///
/// Bodies stay off: headers and timings answer the question, and copying
/// prompts to disk is a decision with a privacy cost that a tool should not
/// make on somebody's behalf.
async fn enable_request_log(desktop: &Desktop) {
    let manage = desktop.app().gproxy().manage();
    let Ok(settings) = manage.settings().get().await else {
        return;
    };
    if settings.logging.enable_downstream_log || settings.logging.enable_upstream_log {
        return;
    }
    let patch = gproxy_sdk::dto::SettingsPatch {
        instance: None,
        logging: Some(gproxy_sdk::dto::LoggingSettingsPatch {
            enable_downstream_log: Some(true),
            enable_upstream_log: Some(true),
            ..Default::default()
        }),
    };
    match manage.settings().update(patch).await {
        Ok(_) => {
            tracing::info!("request logging enabled for this instance");
            if let Err(error) = desktop.app().sync_now().await {
                tracing::warn!(%error, "the logging setting was written but not published");
            }
        }
        Err(error) => tracing::warn!(%error, "could not enable request logging"),
    }
}

/// `StartError` as a sentence a person can act on.
fn describe(error: &StartError) -> String {
    match error {
        StartError::MasterKeyLost { data_dir, .. } => format!(
            "The master key is missing from the login keychain, so the credentials in {data_dir} \
             cannot be opened. Restore the keychain entry or start over with a fresh data folder."
        ),
        other => other.to_string(),
    }
}

/// The path of `gproxy.toml`.
pub fn config_file(data_dir: &Path) -> PathBuf {
    data_dir.join(config::CONFIG_FILE)
}
