//! Where a process with no terminal writes its log.
//!
//! A menu bar app launched by `launchd` has no standard error anybody will
//! ever read, so the log goes to `~/Library/Logs/xproxy/`, one file per day,
//! which is where Console.app and "Reveal Data Folder"'s neighbour both look.
//!
//! The writer is non-blocking: the engine's tracing calls happen on request
//! paths, and a disk that stalls must not become a proxy that stalls. The
//! guard has to outlive the process, which is why it is kept in a `OnceLock`
//! rather than dropped at the end of `init`.

use std::path::Path;
use std::sync::OnceLock;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

static GUARD: OnceLock<WorkerGuard> = OnceLock::new();

/// `~/Library/Logs/xproxy`, or the data directory when there is no home.
pub fn log_dir(data_dir: &Path) -> std::path::PathBuf {
    std::env::var_os("HOME")
        .map(|home| {
            std::path::PathBuf::from(home)
                .join("Library/Logs")
                .join("xproxy")
        })
        .unwrap_or_else(|| data_dir.join("logs"))
}

pub fn init(data_dir: &Path) {
    let dir = log_dir(data_dir);
    if let Err(error) = std::fs::create_dir_all(&dir) {
        eprintln!("xproxy: cannot create {}: {error}", dir.display());
        return;
    }
    let appender = tracing_appender::rolling::daily(&dir, "xproxy.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let _ = GUARD.set(guard);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,xproxy=debug,gproxy=info"));

    let installed = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .with_target(true)
        .try_init();
    if installed.is_err() {
        // A second install is not a failure: the shell may be restarted in a
        // test process that already has a subscriber.
        return;
    }
    tracing::info!(path = %dir.display(), "logging started");
}
