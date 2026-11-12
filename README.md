# xproxy

A local [GPROXY](https://github.com/LeenHawk/gproxy) gateway, managed from the
command line.

GPROXY is a self-hosted gateway for LLM APIs: it holds your upstream accounts —
API keys, the OAuth logins behind coding agents, browser cookies — and gives
everything else one base URL, one key and model names you choose. It ships as a
server, a container, a Cloudflare Worker, and a desktop app whose window is a
full web console.

xproxy is the same engine with a smaller front door: a crate and a binary. No
window, no browser, no console — twelve commands that do what the console does,
and a `serve` that runs the gateway in the foreground.

```text
  xproxy serve ──────────────────────────────┐
                                             │
  xproxy provider|account|route|key|login ───┤
                                             ▼
                                 crate::engine::Gateway
                                             │
                    gproxy_host_tauri::Desktop ── one App, one snapshot,
                                             │      one SQLite file
                             data plane on 127.0.0.1:8787
                                             │
             Claude Code · Codex CLI · anything speaking OpenAI, Claude
             or Gemini
```

## Install

```sh
git clone https://github.com/morganID/xproxy
cd xproxy
./scripts/fetch-core.sh      # clones the engine at the pinned commit
cargo build --release -p xproxy
./target/release/xproxy status
```

`cargo install --path crates/xproxy` works too, and puts `xproxy` on
your `PATH`.

The engine is not vendored and not forked: `gproxy-ref/` is the upstream
repository at a commit pinned in `scripts/fetch-core.sh`, referenced by path.
The first build compiles it — a few hundred crates, so give it a few minutes.

## Commands

```sh
xproxy channels                  # the ids to pass to --channel
xproxy status                    # what is in this instance
xproxy serve                     # run the gateway until Ctrl-C
```

```sh
xproxy provider add --channel openai --name work --api-key sk-...
xproxy provider list
xproxy provider rm <id>          # also removes its accounts

xproxy account add --provider <id> --api-key sk-... --label backup
xproxy account list              # status and how long each token has left
xproxy account rm <id>

xproxy route add --name fast --strategy failover
xproxy route list
xproxy route rm <id>

xproxy key new --name laptop     # printed once, never stored in the clear
xproxy key list
xproxy key rm <id>
```

```sh
xproxy login --provider <id> --label work    # device code or browser redirect
xproxy refresh [--provider <id>] [--force]   # renew tokens now
```

```sh
xproxy models                    # the catalog, from disk
xproxy models --refresh          # ask every provider again
xproxy models --provider <id>    # ask one
xproxy test --model "Google AI Studio/gemma-4-26b-a4b-it" --prompt "Say ok"
```

```sh
xproxy usage --days 7
xproxy log [--errors] [--limit 20]
```

## Signing in

`xproxy login` uses whichever flow the channel offers. A device-code channel
(Cline, WorkOS) prints a short code and opens the browser; a browser-redirect
channel asks for the address the browser landed on. Either way it writes an
account and removes any `dead` ones for that provider — a credential the
upstream refused is not worth keeping as a candidate.

## Tokens

The engine refreshes **on use**: before a request whose token is five minutes
from expiring, before a quota probe, and again if an upstream rejects one. It
has no timer of its own, so an instance that serves nothing renews nothing.

- `xproxy serve` runs the check **every six hours**, and the CLI's `refresh`
  does it on demand. `--force` renews regardless of the expiry.
- `xproxy account list` prints how long each access token has left.
- `status == dead` means the upstream refused the credential: no refresh will
  bring it back, and `xproxy login` is the only cure.

## The model catalog

`xproxy models --refresh` asks each provider for its model list — a real
`GET` to the upstream's directory endpoint, paginated where the API pages —
and writes the answer to `models.json` beside the database. Plain `xproxy
models` reads that file, so listing costs no network and works offline.

It is a cache, not the routing table: `provider_models` in the database means
"routed and priced", which a discovery does not prove.

## Where things live

| Path | What |
| --- | --- |
| `~/Library/Application Support/dev.xproxy/` | `gproxy.db`, `gproxy.toml`, `models.json`, `files/` |
| `~/Library/Logs/xproxy/xproxy.log.<date>` | one file per day |
| login keychain, service `com.leenhawk.gproxy.desktop` | the master key and the gateway key |

`XPROXY_DATA_DIR` points at a different data folder, which is how a second
instance on one machine is possible.

The master key is sealed to the keychain entry, not to the folder: moving the
folder keeps working, copying it to another Mac does not.

## Pointing a client at it

```sh
xproxy serve
# base url  http://127.0.0.1:8787
# key       sk-...
```

```sh
export ANTHROPIC_BASE_URL="http://127.0.0.1:8787"
export ANTHROPIC_AUTH_TOKEN="sk-..."
```

Anything that speaks the OpenAI shape uses the same base URL and key with `/v1`
paths. Model names are `provider/model`, which `xproxy models` prints and
`xproxy test` proves.

## Using it as a library

```rust
let gateway = xproxy::Gateway::local()?;
let status = gateway.start().await?;
println!("{}", status.base_url.unwrap_or_default());

let desktop = gateway.desktop().await?;
let data = desktop.app().data();
let operations = desktop.operations(&data);
let users = operations.users().list(Default::default()).await?;
```

`Gateway` is the engine's lifetime as a value — start it, ask it for the
`Desktop`, stop it — which is the one thing upstream's `OnceLock` engine cannot
do.

## Channels

Every channel that speaks to an upstream over ordinary TLS is compiled in:
OpenAI, Claude API, Codex, DeepSeek, OpenRouter, GLM, Kimi, MiniMax, xAI,
NVIDIA, Azure, Vertex, Bedrock, AI Studio, DashScope, Volcengine, Cloudflare AI
Gateway, Vercel, Cline, Devin, Trae, WorkBuddy and others.

The six that emulate a browser's TLS fingerprint — `claudecode`, `claudeweb`,
`geminicli`, `copilotcli`, `kiro`, `antigravity` — are off by default: they pull
`btls-sys`, whose git dependency carries BoringSSL as a submodule, and cargo
fetches that submodule's entire history. To enable them, add the names to the
`gproxy` features in `Cargo.toml`, restore the `btls-sys` patch line documented
there, and run `scripts/fetch-boringssl.sh` before the first build.

## Keeping up with upstream

The pin is a full commit SHA in `scripts/fetch-core.sh`:

```sh
./scripts/fetch-core.sh --update
cargo check -p xproxy
```

Two things must move with it: the `[patch.crates-io]` block in `Cargo.toml`
(`[patch]` is workspace-local, so upstream's does not reach us — without it the
build fails with `no method named execute_batch`), and the `btls-sys` line in
the comment there.

## Licence

AGPL-3.0-or-later, because it links GPROXY, which is. See `LICENSE`.
