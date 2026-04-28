# Bot Module

Runs mjai-protocol AI bots and routes `MjaiEvent`s from the platform
bridge to them. 

## Submodules

- `types` — `BotResponse`. Wire shape matches mjai.app: an mjai action
  with an optional free-form `meta` JSON object for HUD-grade data
  (confidence, reasoning, q-values, …). Backend never interprets `meta`;
  the frontend renders it.
- `manifest` — per-bot `manifest.toml` schema + `settings.toml` values.
  See "Per-bot settings" below.
- `registry` — discovers bot directories under `mjai_bot/`. No Python
  invocation; only filesystem layout. Populates `BotEntry.manifest` from
  any `manifest.toml` found in the bot directory.
- `runtime` — `PythonRuntime`: locates bundled `python-build-standalone`
  + `uv` (or falls back to system `python3`/`uv`); runs `uv sync` on
  demand against a bot's `pyproject.toml`; produces a `tokio::process::Command`
  ready to spawn the bot. Sync is idempotent via a `mtime:size` stamp at
  `<bot>/.akagi/synced.stamp`.
- `runner` — `BotRunner` async trait + `SubprocessBot` impl. Talks JSONL
  over stdin/stdout, pumps stderr into `tracing` (`bot=<name>` field),
  enforces a 5 s default react timeout, and `kill_on_drop(true)` so a
  dropped runner can't leak children. `reset()` writes `[{"end_game"}]`,
  waits 500 ms, then SIGKILL's and respawns.
- `manager` — `BotManager`: subscribes to the `MjaiBus`, accumulates
  events between decision points (own tsumo / others' dahai-or-kakan /
  reach_accepted / hora / ryukyoku / end_kyoku / end_game), flushes the
  pending batch through the `BotRunner`, and broadcasts every
  `BotResponse` (including `MjaiEvent::None`) on the `BotResponseBus`.
  Spawn point is `start_game` carrying the bot's seat in the `id` field.

## Adding a new bot

Drop a folder under `mjai_bot/`:

```
mjai_bot/<name>/
├── bot.py            # JSONL stdin → JSONL stdout
├── pyproject.toml    # uv-resolved deps; requires-python = ">=3.12"
├── manifest.toml     # OPTIONAL — settings schema (see below)
├── settings.toml     # OPTIONAL — current values; gitignored, written by Akagi
└── README.md         # bot-specific notes (model paths, license, etc.)
```

`bot.py` MUST:

- read one JSON array per line from stdin (a batch of `MjaiEvent`s)
- write one JSON object per line to stdout (a single mjai action — use
  `{"type":"none"}` when no action is owed)
- print **only** protocol JSON to stdout; logs go to stderr
- exit cleanly when it sees `{"type":"end_game"}` in a batch

The bot's seat is delivered three ways (pick whichever fits):

- **argv:** `python bot.py <player_id>` — matches mjai.app convention,
  so unmodified mjai.app bots run as-is.
- **env:** `AKAGI_PLAYER_ID=<player_id>`.
- **`start_game.id`:** the field on the first `start_game` event.

## Per-bot settings

A bot can ship a `manifest.toml` declaring its configurable knobs (API
URLs, keys, model selection, …). The frontend reads the manifest and
renders a generic settings form; the user's edits are persisted in
`settings.toml`.

Example `manifest.toml`:

```toml
manifest_version = 1

[bot]
name        = "mortal"
display     = "Mortal"
description = "AGPL deep-RL mahjong bot."
version     = "0.5.0"

[settings.api_url]
type    = "string"
label   = "API Server URL"
default = "https://api.example.com"
help    = "Endpoint for online inference. Leave blank to run offline."

[settings.api_key]
type    = "string"
label   = "API Key"
secret  = true
default = ""

[settings.online]
type    = "bool"
label   = "Online Mode"
default = false

[settings.temperature]
type    = "float"
label   = "Sampling Temperature"
default = 1.0
min     = 0.0
max     = 2.0
step    = 0.05

[settings.model]
type    = "enum"
label   = "Model"
default = "mortal.pth"
choices = ["mortal.pth", "mortal-1.5.pth"]
```

Field types: `string`, `bool`, `int`, `float`, `enum` (one of `choices`).
`secret = true` makes the frontend render a password input and Akagi
substitutes the value with `***` in tracing.

Bots see the resolved settings (defaults ⊕ on-disk values, validated
against the manifest) via the env var `AKAGI_BOT_CONFIG`, which points
at a JSON file like:

```json
{
  "api_url": "https://api.example.com",
  "online": true
}
```

The bot script can `json.load(open(os.environ["AKAGI_BOT_CONFIG"]))`.
Bots that don't read the env var simply ignore it.

### Secrets caveat

For v1, `secret = true` only changes the *rendering* and *log
substitution* — the value is still stored in `settings.toml`. We
`.gitignore` `settings.toml` so it doesn't end up in source control.
OS keychain integration is on the roadmap; until then, treat
`settings.toml` as a credential file.

### When changes take effect

Updating settings does **not** restart the running bot subprocess. The
new values take effect on the next `start_game` event. The frontend
should warn the user accordingly.

## Why subprocess

- AGPL bots (Mortal et al.) link via stdin/stdout pipe only — arms-length,
  not derivative work. See plan §10.
- Crash isolation: a bot SIGSEGV does not take Akagi down.
- Hot model swap = kill + spawn, no in-process state to flush.

In-process embedding (PyO3) was considered and rejected: linking
libpython makes single-binary distribution painful on Windows/macOS and
couples Akagi's lifecycle to libriichi's.

## What's NOT here

- Mortal weights. Users place `mortal.pth` inside their `mjai_bot/mortal/`
  folder; Akagi never ships, fetches, or configures weight paths. The
  bot script loads them itself.
- 3-player support. v3 is 4P-only for now.
- HUD rendering. `BotResponse`s land on the broadcast bus; the HUD layer
  is a downstream consumer added later.
