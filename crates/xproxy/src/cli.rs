//! The command line: everything that changes the instance.
//!
//! The menu bar reports and pauses. Configuration lives here, because it is
//! the surface that can express it — a provider has a channel, a name, a base
//! URL, a key and possibly a login, and four native dialogs for that is four
//! dialogs too many when a terminal is one command away.
//!
//! # One instance, opened without a socket
//!
//! Every command opens the same database the menu bar app has open, through
//! `instance::open` in `management` mode: no background sync, no listener, no
//! second copy of anything. That is what lets `xproxy provider add` work while
//! the app is running — the app notices on its next poll, and immediately if
//! it was the one that asked.
//!
//! The master key still comes from the login keychain, so a command run as a
//! different user, or after the keychain entry was removed, fails with the
//! same explanation the app would give.

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use gproxy::instance::{self, OpenOptions};
use gproxy_app::dto::{ApiKeyWrite, ListQuery as AppListQuery};
use gproxy_sdk::dto::{CredentialWrite, ListQuery, LogQuery, ProviderWrite, RouteWrite, UsageQuery};
use gproxy_host_tauri::secrets::{self, Keychain};

use crate::error::{Error, Result};

#[derive(Parser)]
#[command(
    name = "xproxy",
    arg_required_else_help = true,
    version,
    about = "A local GPROXY gateway, managed from the command line",
    long_about = "GPROXY holds your upstream accounts and gives everything else \
                  one base URL, one key and model names you choose.\n\n\
                  `xproxy serve` runs the gateway in the foreground; every \
                  other subcommand changes the instance it serves."
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List the channels this build can speak
    Channels,
    /// Upstream providers
    Provider {
        #[command(subcommand)]
        command: ProviderCommand,
    },
    /// Upstream accounts: API keys and logins
    Account {
        #[command(subcommand)]
        command: AccountCommand,
    },
    /// Model names clients ask for
    Route {
        #[command(subcommand)]
        command: RouteCommand,
    },
    /// Keys a client can carry
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },
    /// Requests, tokens and spend
    Usage {
        /// How many days back to count, including today
        #[arg(long, default_value_t = 1)]
        days: u32,
    },
    /// Recent requests, or only the failures
    Log {
        /// Only requests that failed
        #[arg(long)]
        errors: bool,
        /// How many rows
        #[arg(long, default_value_t = 20)]
        limit: u64,
    },
    /// Renew the accounts whose tokens are about to expire
    Refresh {
        /// Only this provider's accounts
        #[arg(long)]
        provider: Option<String>,
        /// Renew now, whatever the expiry says
        #[arg(long)]
        force: bool,
    },
    /// The model catalog: what every provider serves
    Models {
        /// Only this provider, from `xproxy provider list`
        #[arg(long)]
        provider: Option<String>,
        /// Re-ask the upstreams and rewrite the catalog
        #[arg(long)]
        refresh: bool,
    },
    /// Send one request through the gateway, as a client would
    Test {
        /// Model to ask for: `provider/model`, or a route name
        #[arg(long)]
        model: String,
        /// What to say to it
        #[arg(long, default_value = "Reply with the single word: ok")]
        prompt: String,
        /// Output budget. Reasoning models spend it on thinking first, which
        /// is why a small number can return an empty reply.
        #[arg(long, default_value_t = 512)]
        max_tokens: u32,
    },
    /// Sign in to a provider with the flow its channel offers
    Login {
        /// Provider id, from `xproxy provider list`
        #[arg(long)]
        provider: String,
        /// What to call the account this creates
        #[arg(long)]
        label: Option<String>,
    },
    /// Run the gateway in the foreground until interrupted
    Serve,
    /// What this instance is and what is in it
    Status,
}

#[derive(Subcommand)]
enum ProviderCommand {
    /// List providers
    List,
    /// Add a provider, and its API key if one is given
    Add {
        /// Channel id, from `xproxy channels`
        #[arg(long)]
        channel: String,
        /// Name clients will see
        #[arg(long)]
        name: String,
        /// Override the channel's own address
        #[arg(long)]
        base_url: Option<String>,
        /// Store this API key as the provider's first account
        #[arg(long)]
        api_key: Option<String>,
        /// Label for that account
        #[arg(long)]
        label: Option<String>,
    },
    /// Delete a provider and every account under it
    Rm {
        /// Provider id, from `xproxy provider list`
        id: String,
    },
}

#[derive(Subcommand)]
enum AccountCommand {
    /// List accounts
    List {
        /// Only this provider's
        #[arg(long)]
        provider: Option<String>,
    },
    /// Add an API key account
    Add {
        /// Provider id
        #[arg(long)]
        provider: String,
        /// The key itself
        #[arg(long)]
        api_key: String,
        /// What to call it
        #[arg(long)]
        label: Option<String>,
    },
    /// Remove an account
    Rm {
        /// Account id, from `xproxy account list`
        id: String,
    },
}

#[derive(Subcommand)]
enum RouteCommand {
    /// List routes
    List,
    /// Add a route
    Add {
        /// The model name clients send
        #[arg(long)]
        name: String,
        /// round_robin, weighted or failover
        #[arg(long, default_value = "failover")]
        strategy: String,
    },
    /// Delete a route
    Rm { id: String },
}

#[derive(Subcommand)]
enum KeyCommand {
    /// List client keys
    List,
    /// Mint a key. The token is printed once and never again.
    New {
        #[arg(long, default_value = "default")]
        name: String,
    },
    /// Revoke a key
    Rm { id: String },
}

/// Run the command line, and return the process's exit code.
pub fn main() -> i32 {
    let cli = Cli::parse();
    // Every command logs to the daily file: a command that fails at 3am in a
    // cron job leaves the same trail as one that fails in a terminal.
    if let Ok(dir) = data_dir() {
        crate::logging::init(&dir);
    }
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("xproxy: {error}");
            return 1;
        }
    };
    match runtime.block_on(run(cli)) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("xproxy: {error}");
            1
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Channels => channels().await,
        Command::Status => status().await,
        Command::Provider { command } => provider(command).await,
        Command::Account { command } => account(command).await,
        Command::Route { command } => route(command).await,
        Command::Key { command } => key(command).await,
        Command::Refresh { provider, force } => refresh(provider, force).await,
        Command::Models { provider, refresh } => models(provider, refresh).await,
        Command::Test {
            model,
            prompt,
            max_tokens,
        } => test(model, prompt, max_tokens).await,
        Command::Login { provider, label } => login(provider, label).await,
        Command::Serve => serve().await,
        Command::Usage { days } => usage(days).await,
        Command::Log { errors, limit } => log(errors, limit).await,
    }
}

// ------------------------------------------------------------------ open --

/// One open instance, without a socket.
struct Open {
    app: Arc<gproxy_app::App<instance::Connection>>,
}

/// Open the instance the menu bar app uses.
///
/// The path is the one Tauri resolves from the bundle identifier, computed here
/// because a command has no `AppHandle` to ask. `XPROXY_DATA_DIR` overrides it,
/// which is what makes a second instance on the same machine possible.
fn data_dir() -> Result<PathBuf> {
    crate::engine::data_dir()
}

async fn open() -> Result<Open> {
    let dir = data_dir()?;
    // `fresh` is asked before anything is opened, because opening creates the
    // database and the question cannot be answered afterwards.
    let fresh = !dir.join("gproxy.db").is_file() && !dir.join("secrets.json").is_file();
    let master_key = secrets::master_key(&Keychain, &dir, fresh)?;
    let settings = gproxy_host_tauri::config::settings(&dir, master_key)?;
    let instance = instance::open(&settings, OpenOptions::management()).await?;
    Ok(Open { app: instance.app })
}

impl Open {
    /// Publish what was just written, so the running app sees it on the next
    /// request rather than on its next poll.
    async fn settle(&self) {
        if let Err(error) = self.app.sync_now().await {
            eprintln!("xproxy: warning: the snapshot was not refreshed: {error}");
        }
    }
}

// ------------------------------------------------------------- commands --

async fn channels() -> Result<()> {
    let open = open().await?;
    let channels = open.app.gproxy().manage().catalog().channels();
    for channel in channels {
        let kinds = channel.credential_kinds.join(", ");
        println!("{:<22} {:<28} {}", channel.id, channel.display_name, kinds);
    }
    Ok(())
}

async fn status() -> Result<()> {
    let open = open().await?;
    let providers = open
        .app
        .gproxy()
        .manage()
        .providers()
        .list(list())
        .await?;
    let accounts = open
        .app
        .gproxy()
        .manage()
        .credentials()
        .list(list())
        .await?;
    let routes = open.app.gproxy().manage().routes().list(list()).await?;
    let summary = open
        .app
        .gproxy()
        .query()
        .usage()
        .summary(today())
        .await?;

    println!("data      {}", data_dir()?.display());
    println!("providers {}", providers.total);
    println!("accounts  {}", accounts.total);
    println!("routes    {}", routes.total);
    println!(
        "today     {} requests · {} tokens · {}",
        summary.requests,
        summary.input_tokens + summary.output_tokens,
        summary.cost
    );
    Ok(())
}

async fn provider(command: ProviderCommand) -> Result<()> {
    let open = open().await?;
    match command {
        ProviderCommand::List => {
            let page = open
                .app
                .gproxy()
                .manage()
                .providers()
                .list(list())
                .await?;
            if page.items.is_empty() {
                println!("no providers yet — add one with `xproxy provider add`");
                return Ok(());
            }
            for provider in page.items {
                println!(
                    "{:<12} {:<24} {:<16} {}",
                    provider.id,
                    provider.display_name.unwrap_or(provider.name),
                    provider.channel,
                    provider.base_url.unwrap_or_default()
                );
            }
        }
        ProviderCommand::Add {
            channel,
            name,
            base_url,
            api_key,
            label,
        } => {
            let write = ProviderWrite {
                name: name.clone(),
                display_name: Some(name),
                channel: channel.clone(),
                base_url,
                enabled: Some(true),
                ..Default::default()
            };
            let created = open
                .app
                .gproxy()
                .manage()
                .providers()
                .create(write)
                .await?;
            println!("provider {} ({channel})", created.id);
            if let Some(key) = api_key {
                let write = CredentialWrite {
                    provider_id: created.id.clone(),
                    label: label.or_else(|| Some("api key".to_owned())),
                    auth_kind: "api_key".to_owned(),
                    secret: serde_json::json!({ "api_key": key }),
                    enabled: Some(true),
                    ..Default::default()
                };
                let account = open
                    .app
                    .gproxy()
                    .manage()
                    .credentials()
                    .create(write)
                    .await?;
                println!("account  {} (api_key)", account.id);
            }
            open.settle().await;
        }
        ProviderCommand::Rm { id } => {
            open.app
                .gproxy()
                .manage()
                .providers()
                .delete(&id)
                .await?;
            println!("deleted {id}");
            open.settle().await;
        }
    }
    Ok(())
}

async fn account(command: AccountCommand) -> Result<()> {
    let open = open().await?;
    match command {
        AccountCommand::List { provider } => {
            let mut query = list();
            query.provider_id = provider;
            let page = open
                .app
                .gproxy()
                .manage()
                .credentials()
                .list(query)
                .await?;
            if page.items.is_empty() {
                println!("no accounts yet — add one with `xproxy account add`");
                return Ok(());
            }
            for account in page.items {
                println!(
                    "{:<12} {:<12} {:<10} {:<10} {:<18} {}",
                    account.id,
                    account.provider_id,
                    account.auth_kind,
                    account.status,
                    crate::util::remaining(account.expires_at_ms),
                    account.label.unwrap_or_default()
                );
            }
        }
        AccountCommand::Add {
            provider,
            api_key,
            label,
        } => {
            let write = CredentialWrite {
                provider_id: provider,
                label: label.or_else(|| Some("api key".to_owned())),
                auth_kind: "api_key".to_owned(),
                secret: serde_json::json!({ "api_key": api_key }),
                enabled: Some(true),
                ..Default::default()
            };
            let created = open
                .app
                .gproxy()
                .manage()
                .credentials()
                .create(write)
                .await?;
            println!("account {} (api_key)", created.id);
            open.settle().await;
        }
        AccountCommand::Rm { id } => {
            open.app
                .gproxy()
                .manage()
                .credentials()
                .delete(&id)
                .await?;
            println!("deleted {id}");
            open.settle().await;
        }
    }
    Ok(())
}

async fn route(command: RouteCommand) -> Result<()> {
    let open = open().await?;
    match command {
        RouteCommand::List => {
            let page = open.app.gproxy().manage().routes().list(list()).await?;
            if page.items.is_empty() {
                println!("no routes yet — add one with `xproxy route add --name fast`");
                return Ok(());
            }
            for route in page.items {
                println!(
                    "{:<12} {:<24} {:<12} {}",
                    route.id,
                    route.name,
                    route.strategy,
                    if route.enabled { "enabled" } else { "off" }
                );
            }
        }
        RouteCommand::Add { name, strategy } => {
            let write = RouteWrite {
                name: name.clone(),
                strategy: Some(strategy.clone()),
                enabled: Some(true),
                ..Default::default()
            };
            let created = open.app.gproxy().manage().routes().create(write).await?;
            println!("route {} ({name}, {strategy})", created.id);
            open.settle().await;
        }
        RouteCommand::Rm { id } => {
            open.app.gproxy().manage().routes().delete(&id).await?;
            println!("deleted {id}");
            open.settle().await;
        }
    }
    Ok(())
}

async fn key(command: KeyCommand) -> Result<()> {
    let open = open().await?;
    match command {
        KeyCommand::List => {
            let data = open.app.data();
            let operations = operations(&open, &data);
            let page = operations.api_keys().list(app_list()).await?;
            if page.items.is_empty() {
                println!("no client keys yet — add one with `xproxy key new`");
                return Ok(());
            }
            for key in page.items {
                println!("{:<12} {:<24} {}", key.id, key.name, key.enabled);
            }
        }
        KeyCommand::New { name } => {
            let user_id = administrator(&open).await?;
            let data = open.app.data();
            let created = operations(&open, &data)
                .api_keys()
                .create(ApiKeyWrite {
                    id: None,
                    user_id,
                    name,
                    organization_id: None,
                    team_id: None,
                    expires_at_ms: None,
                    enabled: Some(true),
                    retain_secret: Some(false),
                    management: None,
                    budget: None,
                })
                .await?;
            println!("{}", created.token);
            eprintln!("this key is shown once; it is not stored in the clear");
            open.settle().await;
        }
        KeyCommand::Rm { id } => {
            let data = open.app.data();
            operations(&open, &data).api_keys().delete(&id).await?;
            println!("revoked {id}");
            open.settle().await;
        }
    }
    Ok(())
}

/// Renew tokens now, instead of waiting for the six-hour tick or for a request
/// to notice they were about to die.
///
/// `IfNeeded` by default: the engine's own rule — five minutes before the
/// access token expires, a day before the refresh token does — decides, so an
/// account that is fine costs one indexed read. `--force` renews regardless,
/// which is what to run when an upstream has invalidated a token early.
async fn refresh(provider: Option<String>, force: bool) -> Result<()> {
    let open = open().await?;
    let _ = provider;
    let outcomes = crate::refresh::accounts(&open.app, force).await?;
    if outcomes.is_empty() {
        println!("no accounts to refresh");
        return Ok(());
    }
    let mut failures = 0;
    for outcome in outcomes {
        if let Some(error) = &outcome.failure {
            failures += 1;
            println!("{:<20} failed     {error}", outcome.label);
        } else if outcome.skipped {
            println!("{:<20} {:<10} no refresh for this channel", outcome.label, outcome.status);
        } else {
            println!(
                "{:<20} {:<10} {}",
                outcome.label,
                outcome.status,
                crate::util::remaining(outcome.expires_at_ms)
            );
        }
    }
    if failures > 0 {
        return Err(Error::new(
            "refresh",
            format!("{failures} account(s) could not be renewed"),
        ));
    }
    Ok(())
}

/// Serve until interrupted, which is what a supervisor or a terminal wants.
///
/// The engine's own binary does this too; this one exists so that a single
/// `cargo install` gives both the management commands and the process that
/// serves them, sharing one data folder and one keychain entry.
async fn serve() -> Result<()> {
    let gateway = crate::Gateway::local()?;
    let status = gateway.start().await?;
    let desktop = gateway.desktop().await?;
    let plane = desktop.data_plane();
    println!("base url  {}", plane.base_url);
    println!("key       {}", plane.gateway_key);
    println!("data      {}", status.data_dir);
    eprintln!("serving; press Ctrl-C to stop");

    // The token timer the engine does not schedule for itself.
    let timer = tokio::spawn({
        let app = desktop.app().clone();
        async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(6 * 60 * 60)).await;
                match crate::refresh::accounts(&app, false).await {
                    Ok(outcomes) => {
                        let failed: Vec<&str> = outcomes
                            .iter()
                            .filter_map(|outcome| outcome.failure.as_deref())
                            .collect();
                        if !failed.is_empty() {
                            tracing::warn!(?failed, "some accounts did not refresh");
                        }
                    }
                    Err(error) => tracing::warn!(%error, "the token check did not run"),
                }
            }
        }
    });

    let _ = tokio::signal::ctrl_c().await;
    timer.abort();
    gateway.stop().await?;
    eprintln!("stopped");
    Ok(())
}

/// The catalog, from disk — or freshly discovered with `--refresh`.
///
/// Discovery is a live call to every upstream and takes seconds per provider,
/// so it is not something a listing command does by default. The answer is
/// written to `models.json` beside the database, which is what the menu reads
/// without touching the network.
async fn models(provider: Option<String>, refresh: bool) -> Result<()> {
    let open = open().await?;
    let dir = data_dir()?;

    let catalog = if refresh || provider.is_some() {
        crate::catalog::discover(&open.app, &dir, provider.as_deref()).await?
    } else {
        crate::catalog::load(&dir)
    };

    if catalog.providers.is_empty() {
        println!("the catalog is empty — run `xproxy models --refresh`");
        return Ok(());
    }

    for entry in catalog.providers.values() {
        if let Some(error) = &entry.error {
            println!("{} ({}) — {error}", entry.name, entry.channel);
            continue;
        }
        println!("{} ({}) · {} models", entry.name, entry.channel, entry.models.len());
        for model in &entry.models {
            println!("  {model}");
        }
    }
    if provider.is_none() {
        let total: usize = catalog.providers.values().map(|entry| entry.models.len()).sum();
        println!("\n{total} models across {} providers", catalog.providers.len());
    }
    Ok(())
}

async fn test(model: String, prompt: String, max_tokens: u32) -> Result<()> {
    let dir = data_dir()?;
    let key = secrets::gateway_key(&Keychain, &dir).ok_or_else(|| {
        Error::new(
            "gateway_key",
            "no gateway key in the keychain; start the app once to mint one",
        )
    })?;

    let body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": [{ "role": "user", "content": prompt }],
    });
    let started = std::time::Instant::now();
    let response = reqwest::Client::new()
        .post("http://127.0.0.1:8787/v1/chat/completions")
        .bearer_auth(&key)
        .json(&body)
        .send()
        .await
        .map_err(|error| {
            Error::new(
                "unreachable",
                format!("the data plane did not answer ({error}); is the app running?"),
            )
        })?;
    let status = response.status();
    let text = response.text().await.map_err(Error::internal)?;
    let elapsed = started.elapsed().as_millis();

    println!("status {status} · {elapsed} ms");
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(value) => {
            if let Some(content) = value
                .pointer("/choices/0/message/content")
                .and_then(|content| content.as_str())
            {
                println!("reply  {content}");
                println!(
                    "usage  {} in · {} out",
                    value.pointer("/usage/prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                    value
                        .pointer("/usage/completion_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                );
            } else {
                // An empty reply with `finish_reason: length` is the most
                // confusing success there is: the model answered, and the
                // answer was all thinking. Say so rather than print a wall of
                // JSON that looks like a failure.
                let reason = value
                    .pointer("/choices/0/finish_reason")
                    .and_then(|reason| reason.as_str())
                    .unwrap_or("");
                if reason == "length" {
                    println!("reply  (empty — the token budget went to thinking; raise --max-tokens)");
                } else {
                    println!("{text}");
                }
            }
        }
        Err(_) => println!("{text}"),
    }
    if !status.is_success() {
        return Err(Error::new("request_failed", format!("the gateway answered {status}")));
    }
    Ok(())
}

/// Sign in, with whichever flow the provider's channel offers.
///
/// Device code is the one that fits a terminal: the upstream prints a short
/// code, the person types it in a browser, and this polls until the account
/// exists. A channel that only speaks authorization-code gets the callback URL
/// pasted back, because a one-shot command cannot wait behind a browser tab
/// that has no server behind it.
async fn login(provider: String, label: Option<String>) -> Result<()> {
    let open = open().await?;
    let channel = open
        .app
        .gproxy()
        .manage()
        .providers()
        .get(&provider)
        .await?
        .channel;
    let modes = open
        .app
        .gproxy()
        .manage()
        .catalog()
        .channels()
        .into_iter()
        .find(|descriptor| descriptor.id == channel)
        .map(|descriptor| descriptor.login_modes)
        .unwrap_or_default();

    if modes.contains(&gproxy_channel::LoginMode::DeviceCode) {
        return device_login(&open, &provider, label).await;
    }
    if modes.contains(&gproxy_channel::LoginMode::AuthorizationCode) {
        return code_login(&open, &provider, label).await;
    }
    Err(Error::new(
        "login",
        format!("`{channel}` has no sign-in flow; add an API key with `xproxy account add`"),
    ))
}

async fn device_login(
    open: &Open,
    provider: &str,
    label: Option<String>,
) -> Result<()> {
    let request = gproxy_sdk::dto::DeviceStart {
        provider_id: provider.to_owned(),
        label,
        owner: Default::default(),
    };
    let started = open.app.gproxy().login().device_start(request).await?;

    println!("code  {}", started.user_code);
    println!("open  {}", started.verification_uri);
    if let Some(complete) = started.verification_uri_complete.clone() {
        println!("      {complete}");
        let _ = crate::util::open_url(&complete);
    } else {
        let _ = crate::util::open_url(&started.verification_uri);
    }
    println!("waiting for approval…");

    loop {
        let outcome = open
            .app
            .gproxy()
            .login()
            .device_poll(&started.login_session_id)
            .await?;
        // The outcome is internally tagged: `{"status": "ready", "credentialId": …}`.
        // Reading it as an externally tagged enum is what made the first
        // version poll past a successful login and create the account twice.
        let value = serde_json::to_value(outcome).map_err(Error::internal)?;
        let status = value
            .get("status")
            .and_then(|status| status.as_str())
            .unwrap_or("pending");
        match status {
            "ready" => {
                let id = value
                    .get("credentialId")
                    .and_then(|id| id.as_str())
                    .unwrap_or("")
                    .to_owned();
                println!("account {id}");
                remove_dead(open, provider, &id).await;
                open.settle().await;
                return Ok(());
            }
            "denied" => return Err(Error::new("login", "the sign-in was refused")),
            "expired" => return Err(Error::new("login", "the code expired; run this again")),
            _ => {}
        }
        let interval = value
            .get("intervalSecs")
            .and_then(|value| value.as_u64())
            .unwrap_or(5);
        tokio::time::sleep(std::time::Duration::from_secs(interval.max(2))).await;
    }
}

async fn code_login(open: &Open, provider: &str, label: Option<String>) -> Result<()> {
    let request = gproxy_sdk::dto::AuthCodeStart {
        provider_id: provider.to_owned(),
        redirect_uri: None,
        label,
        owner: Default::default(),
    };
    let started = open.app.gproxy().login().authcode_start(request).await?;
    println!("open  {}", started.authorize_url);
    println!("      redirects to {}", started.redirect_uri);
    let _ = crate::util::open_url(&started.authorize_url);

    let mut line = String::new();
    eprint!("paste the address the browser landed on: ");
    std::io::stdin()
        .read_line(&mut line)
        .map_err(Error::internal)?;
    let callback = line.trim().to_owned();
    if callback.is_empty() {
        return Err(Error::new("login", "no callback URL given"));
    }

    let request = gproxy_sdk::dto::AuthCodeComplete {
        login_session_id: started.login_session_id,
        callback_url: Some(callback),
        code: None,
        state: None,
    };
    let created = open.app.gproxy().login().authcode_complete(request).await?;
    let value = serde_json::to_value(created).map_err(Error::internal)?;
    let id = value
        .get("id")
        .and_then(|id| id.as_str())
        .unwrap_or("")
        .to_owned();
    println!("account {id}");
    remove_dead(open, provider, &id).await;
    open.settle().await;
    Ok(())
}

/// Drop the accounts this sign-in replaced.
///
/// A dead credential is one the upstream refused for good; signing in again is
/// the only cure, and leaving the corpse in the list means every future request
/// has one more candidate to fail on before it reaches the live one. Only
/// `dead` rows go: an account that is merely disabled, or whose access token
/// expired, is somebody's deliberate choice or a refresh away from working.
async fn remove_dead(open: &Open, provider: &str, keep: &str) {
    let mut query = list();
    query.provider_id = Some(provider.to_owned());
    let Ok(page) = open
        .app
        .gproxy()
        .manage()
        .credentials()
        .list(query)
        .await
    else {
        return;
    };
    for account in page.items {
        if account.id == keep || account.status != "dead" {
            continue;
        }
        match open
            .app
            .gproxy()
            .manage()
            .credentials()
            .delete(&account.id)
            .await
        {
            Ok(_) => println!("removed  {} (was dead)", account.id),
            Err(error) => eprintln!("xproxy: could not remove {}: {error}", account.id),
        }
    }
}

async fn usage(days: u32) -> Result<()> {
    let open = open().await?;
    let from = crate::util::local_midnight_ms() - i64::from(days.saturating_sub(1)) * 86_400_000;
    let summary = open
        .app
        .gproxy()
        .query()
        .usage()
        .summary(UsageQuery {
            from_ms: Some(from),
            ..Default::default()
        })
        .await?;
    println!(
        "{} requests · {} in · {} out · {} cached · {}",
        summary.requests,
        summary.input_tokens,
        summary.output_tokens,
        summary.cached_input_tokens,
        summary.cost
    );
    if let Some(tps) = summary.average_output_tps {
        println!("{tps:.1} output tokens/second");
    }
    Ok(())
}

async fn log(errors: bool, limit: u64) -> Result<()> {
    let open = open().await?;
    let page = open
        .app
        .gproxy()
        .query()
        .logs()
        .list(LogQuery {
            limit: Some(limit.min(500)),
            ..Default::default()
        })
        .await?;
    for entry in page.items {
        let failed = entry.error.is_some()
            || entry.reason.is_some()
            || entry.response_status.is_some_and(|status| status >= 400)
            || entry.state == "failed";
        if errors && !failed {
            continue;
        }
        println!(
            "{}  {:<4} {:<28} {:<10} {}",
            crate::util::clock(entry.started_at_ms),
            entry
                .response_status
                .map(|status| status.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            entry.model.or(entry.operation).unwrap_or_else(|| "-".to_owned()),
            entry.state,
            entry
                .reason
                .or(entry.error)
                .unwrap_or_default()
        );
    }
    Ok(())
}

/// The administrator this instance was created with.
async fn administrator(open: &Open) -> Result<String> {
    let data = open.app.data();
    let page = operations(open, &data).users().list(app_list()).await?;
    page.items
        .into_iter()
        .find(|user| user.role == "admin")
        .map(|user| user.id)
        .ok_or_else(|| Error::new("internal", "this instance has no administrator"))
}

/// `Operations` borrows the snapshot, so it is built at the call site from a
/// `data` local that outlives the call — the one-load-per-request rule both
/// HTTP hosts follow.
fn operations<'a>(
    open: &'a Open,
    data: &'a gproxy_app::AppData,
) -> gproxy_app::Operations<'a, instance::Connection> {
    gproxy_app::Operations::new(open.app.gproxy(), data, open.app.config())
}

fn list() -> ListQuery {
    ListQuery {
        page_size: Some(200),
        ..Default::default()
    }
}

fn app_list() -> AppListQuery {
    AppListQuery {
        page_size: Some(200),
        ..Default::default()
    }
}

fn today() -> UsageQuery {
    UsageQuery {
        from_ms: Some(crate::util::local_midnight_ms()),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use gproxy_sdk::dto::DevicePollOutcome;

    /// The shape this file reads, pinned.
    ///
    /// The first version of the poll loop assumed an externally tagged enum
    /// (`{"ready": {…}}`), so it never saw the login succeed, kept polling, and
    /// created the same account on every poll until the code expired. This test
    /// fails the moment that assumption stops being true again.
    #[test]
    fn the_poll_outcome_is_tagged_by_status() {
        let ready = serde_json::to_value(DevicePollOutcome::Ready {
            credential_id: "c1".to_owned(),
        })
        .unwrap();
        assert_eq!(ready["status"], "ready");
        assert_eq!(ready["credentialId"], "c1");

        let pending = serde_json::to_value(DevicePollOutcome::Pending { interval_secs: 7 }).unwrap();
        assert_eq!(pending["status"], "pending");
        assert_eq!(pending["intervalSecs"], 7);
    }
}
