//! One error shape for every command in this shell.
//!
//! Deliberately the same fields as upstream's `IpcError` — `code`, `message`,
//! `status` is dropped because nothing here is an HTTP request — so that a
//! failure from an operation and a failure from the shell itself render in the
//! popover with one rule.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Error {
    pub code: String,
    pub message: String,
}

impl Error {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn internal(message: impl std::fmt::Display) -> Self {
        Self::new("internal", message.to_string())
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

/// The engine's failures, in this shell's shape.
///
/// Through `AppError` rather than directly, so an engine error is coded the
/// same way whichever surface raised it — the same rule upstream's IPC
/// envelope follows.
impl From<gproxy_app::AppError> for Error {
    fn from(error: gproxy_app::AppError) -> Self {
        Self {
            code: error.code().to_owned(),
            message: error.to_string(),
        }
    }
}

impl From<gproxy_sdk::SdkError> for Error {
    fn from(error: gproxy_sdk::SdkError) -> Self {
        gproxy_app::AppError::from(error).into()
    }
}

/// A failure while opening the instance — a keychain that will not open, a
/// database another process holds, a port already taken.
impl From<gproxy_host_tauri::StartError> for Error {
    fn from(error: gproxy_host_tauri::StartError) -> Self {
        Self {
            code: "start_failed".to_owned(),
            message: error.to_string(),
        }
    }
}

/// The CLI's own configuration errors, which have already been phrased for a
/// person reading a terminal.
impl From<gproxy::Error> for Error {
    fn from(error: gproxy::Error) -> Self {
        Self {
            code: "config".to_owned(),
            message: error.to_string(),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
