//! The model catalog: what each provider actually serves, kept on disk.
//!
//! Asking an upstream for its model list is a network call that takes seconds
//! and can fail. The menu is rebuilt every minute, so it cannot do that — it
//! reads a file. This module owns the file, and the one operation that fills
//! it.
//!
//! # Why a file and not the database
//!
//! The database already has a `provider_models` table, and it means something
//! specific to the engine: a model this provider is *routed* for, with a price
//! rule. A catalog entry is weaker than that — "the upstream says it exists" —
//! and writing it into the routing table would be claiming more than the
//! discovery proved. So it is a cache beside the database, in the same folder,
//! and the menu says how old it is.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gproxy::instance;
use gproxy_app::App;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

const FILE: &str = "models.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    #[serde(default)]
    pub updated_at_ms: i64,
    /// Provider id → what it serves.
    #[serde(default)]
    pub providers: BTreeMap<String, Entry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub channel: String,
    #[serde(default)]
    pub models: Vec<String>,
    /// The last discovery's failure, if it failed. Kept so the menu can say
    /// *why* a provider is missing rather than showing an empty list.
    #[serde(default)]
    pub error: Option<String>,
}

pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE)
}

pub fn load(data_dir: &Path) -> Catalog {
    std::fs::read(path(data_dir))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save(data_dir: &Path, catalog: &Catalog) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(catalog).map_err(Error::internal)?;
    std::fs::write(path(data_dir), bytes).map_err(Error::internal)
}

/// Ask every provider (or one) what it serves, and write the answer down.
///
/// A provider that fails keeps its previous entry and gains an `error`, so one
/// upstream being down does not blank the catalog for the others.
pub async fn discover(
    app: &Arc<App<instance::Connection>>,
    data_dir: &Path,
    only: Option<&str>,
) -> Result<Catalog> {
    let manage = app.gproxy().manage();
    let providers = manage
        .providers()
        .list(gproxy_sdk::dto::ListQuery {
            page_size: Some(200),
            ..Default::default()
        })
        .await?
        .items;

    let mut catalog = load(data_dir);
    catalog.updated_at_ms = now_ms();
    for provider in providers {
        if only.is_some_and(|id| id != provider.id) {
            continue;
        }
        let name = provider
            .display_name
            .clone()
            .unwrap_or_else(|| provider.name.clone());
        let previous = catalog.providers.get(&provider.id).cloned().unwrap_or_default();
        let entry = match manage
            .connectivity()
            .discover_models(&provider.id, None)
            .await
        {
            Ok(found) => Entry {
                name,
                channel: provider.channel,
                models: found
                    .into_iter()
                    .map(|model| model.upstream_name)
                    .collect(),
                error: None,
            },
            Err(error) => Entry {
                name,
                channel: provider.channel,
                error: Some(error.to_string()),
                ..previous
            },
        };
        catalog.providers.insert(provider.id, entry);
    }

    save(data_dir, &catalog)?;
    Ok(catalog)
}

/// Every model as `provider/model`, which is what a client asks for.
pub fn flat(catalog: &Catalog) -> Vec<(String, String, String)> {
    let mut rows = Vec::new();
    for entry in catalog.providers.values() {
        for model in &entry.models {
            rows.push((entry.name.clone(), model.clone(), format!("{}/{}", entry.name, model)));
        }
    }
    rows.sort_by(|a, b| a.2.cmp(&b.2));
    rows
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
