//! Renewing credentials, on demand and on a timer.
//!
//! The engine refreshes *on use*: before a request whose token is five minutes
//! from expiring, before a quota probe, and again if an upstream rejects one.
//! That is the right design for something always serving — nothing is
//! refreshed that nobody needs — and the wrong one for a laptop that idles for
//! a week, where nothing is used and so nothing is renewed.
//!
//! [`accounts`] is the timer's body: it asks every refreshable account whether
//! it is due, and the engine's own rule answers (five minutes before the
//! access token dies, a day before the refresh token does).

use std::sync::Arc;

use gproxy::instance;
use gproxy_app::App;
use gproxy_core::RefreshMode;
use gproxy_sdk::dto::ListQuery;

use crate::error::Result;

/// One account's turn at the token check.
pub struct RefreshOutcome {
    pub label: String,
    pub status: String,
    pub expires_at_ms: Option<i64>,
    /// The channel has no refresh ability — an API key has nothing to renew,
    /// and counting it as a failure would make every check look broken.
    pub skipped: bool,
    pub failure: Option<String>,
}

/// Refresh the accounts whose tokens are close to expiring.
pub async fn accounts(
    app: &Arc<App<instance::Connection>>,
    force: bool,
) -> Result<Vec<RefreshOutcome>> {
    let manage = app.gproxy().manage();

    // Which channels can refresh at all, so an API key is skipped rather than
    // reported as a broken OAuth login.
    let refreshable: Vec<String> = manage
        .catalog()
        .channels()
        .into_iter()
        .filter(|descriptor| descriptor.capabilities.refresh)
        .map(|descriptor| descriptor.id.to_owned())
        .collect();
    let channels: Vec<(String, String)> = manage
        .providers()
        .list(page())
        .await?
        .items
        .into_iter()
        .map(|provider| (provider.id, provider.channel))
        .collect();

    let page = manage.credentials().list(page()).await?;
    let mode = if force {
        RefreshMode::Force
    } else {
        RefreshMode::IfNeeded
    };

    let mut outcomes = Vec::new();
    let mut changed = false;
    for account in page.items {
        let label = account
            .label
            .clone()
            .unwrap_or_else(|| account.id.clone());
        let channel = channels
            .iter()
            .find(|(id, _)| *id == account.provider_id)
            .map(|(_, channel)| channel.as_str())
            .unwrap_or_default();
        if !refreshable.iter().any(|id| id == channel) {
            outcomes.push(RefreshOutcome {
                label,
                status: account.status,
                expires_at_ms: account.expires_at_ms,
                skipped: true,
                failure: None,
            });
            continue;
        }
        match manage.credentials().refresh(&account.id, mode).await {
            Ok(refreshed) => {
                changed = true;
                outcomes.push(RefreshOutcome {
                    label,
                    status: refreshed.status,
                    expires_at_ms: refreshed.expires_at_ms,
                    skipped: false,
                    failure: None,
                });
            }
            Err(error) => outcomes.push(RefreshOutcome {
                label,
                status: account.status,
                expires_at_ms: account.expires_at_ms,
                skipped: false,
                failure: Some(error.to_string()),
            }),
        }
    }

    if changed {
        if let Err(error) = app.sync_now().await {
            tracing::warn!(%error, "the refreshed credentials were not published");
        }
    }
    Ok(outcomes)
}

fn page() -> ListQuery {
    ListQuery {
        page_size: Some(200),
        ..Default::default()
    }
}
