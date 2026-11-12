//! The two small things the CLI does to the machine that are not the engine.

use crate::error::{Error, Result};

/// Open a URL in the default browser, which is where every login begins.
pub fn open_url(url: &str) -> Result<()> {
    std::process::Command::new("/usr/bin/open")
        .arg(url)
        .status()
        .map_err(Error::internal)?;
    Ok(())
}

/// How long an access token has left: "42 min left", "3 h left", "expired".
pub fn remaining(expires_at_ms: Option<i64>) -> String {
    let Some(expires) = expires_at_ms else {
        return String::new();
    };
    let left = expires - crate::now_ms();
    if left <= 0 {
        return "expired".to_owned();
    }
    let minutes = left / 60_000;
    if minutes < 90 {
        format!("{minutes} min left")
    } else {
        format!("{} h left", minutes / 60)
    }
}

/// Local midnight, in milliseconds.
///
/// `date` rather than arithmetic: "today" is a question about the machine's
/// time zone, and `-v0H -v0M -v0S` is the system's own answer to it.
pub fn local_midnight_ms() -> i64 {
    let output = std::process::Command::new("/bin/date")
        .args(["-v0H", "-v0M", "-v0S", "+%s"])
        .output();
    match output {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<i64>()
            .map(|seconds| seconds * 1000)
            .unwrap_or_else(|_| crate::now_ms() - 86_400_000),
        _ => crate::now_ms() - 86_400_000,
    }
}

/// `14:03`, in the machine's zone.
pub fn clock(ms: i64) -> String {
    let output = std::process::Command::new("/bin/date")
        .args(["-r", &(ms / 1000).to_string(), "+%H:%M"])
        .output();
    match output {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        }
        _ => String::new(),
    }
}
