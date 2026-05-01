# Akagi v3

Mahjong AI assistant for Majsoul. A Tauri desktop app that intercepts game
traffic via a local MITM proxy, mirrors the game state, runs an mjai-protocol
bot, and renders live discard / risk analysis in the webview.

> Status: in active development on the `v3` branch. Rewrite of
> [Akagi](https://github.com/shinkuan/Akagi) (Python) and
> [AkagiNG](https://github.com/shinkuan/AkagiNG) (Electron + Python) into a
> single Rust binary with a static-HTML frontend.

---

## Highlights

- **Single Rust binary.** Tauri shell, MITM proxy, protocol bridge, game-state
  tracker, analysis engine, bot supervisor — all in-process.
- **Pluggable mjai bots.** Drop a folder under `mjai_bot/`; Akagi locates a
  bundled `python-build-standalone` + `uv`, runs `uv sync` once, and pipes
  JSONL over stdin/stdout. Convention is identical to mjai.app.
- **Analysis engine.** Rust port of `mahjong-helper/util/` on top of
  `riichienv-core` — shanten, waits, agari-rate, tenpai-rate, risk vector,
  best attack / defence discard.
- **Live frontend.** Six push events (mjai, bot response, bot/proxy lifecycle,
  notifications, analysis) and eleven pull commands. Pre-encoded mahgen DSL
  strings ship straight to the `<mah-gen>` Web Component.

---

## Architecture

```
                ┌────────────────────────┐
   Majsoul ──── │  proxy (hudsucker)     │ ── CA at ./ca
   WebSocket    └─────────┬──────────────┘
                          ▼
                ┌────────────────────────┐
                │  bridge::majsoul       │   liqi protobuf → MjaiEvent
                └─────────┬──────────────┘
                          ▼ MjaiBus
       ┌──────────────────┼──────────────────┐
       ▼                  ▼                  ▼
  game_state::tracker   bot::manager     ipc forwarder
       │                  │                  │
       ▼ PostBus          ▼ BotResponseBus   ▼ app.emit
  analysis::runner   subprocess (uv)    Tauri webview
       │
       ▼ AnalysisBus
       └──► ipc forwarder ──► app.emit
```

`src/lib.rs` wires the buses; subsystems own only their bus handles, never
each other. `src/event_bus.rs` is the single source of truth for channel
types.

---

## Project layout

```
.
├── src/
│   ├── proxy/        MITM HTTP/HTTPS/WS via hudsucker; CA at ./ca
│   ├── bridge/       Per-platform protocol → MjaiEvent (currently majsoul/)
│   ├── schema/       MjaiEvent enum + IPC payload types
│   ├── game_state/   riichienv-driven state mirror, snapshot, mahgen view
│   ├── analysis/     shanten / waits / risk / discard search
│   ├── bot/          registry, python runtime, JSONL subprocess runner
│   ├── ipc/          Tauri commands, app state, proxy supervisor
│   ├── config/       AppConfig (TOML) sections
│   ├── event_bus.rs  Broadcast channels between subsystems
│   ├── logger/       Per-session log dir + per-target file appenders
│   └── lib.rs        Boot / wiring
├── mjai_bot/
│   └── example/      Rule-based shanten optimizer (ships in tree)
├── frontend/         Static HTML/CSS/JS served by Tauri
├── tests/            Integration tests (analysis pipeline, bot lifecycle, …)
├── capabilities/     Tauri permissions
├── icons/            App icons
├── tauri.conf.json   Window + bundle config
└── Cargo.toml
```

Per-module developer guides live in each `src/*/README.md`.

---

## Build & run

Prerequisites: Rust (latest stable), Tauri 2 deps (webkit2gtk on Linux).

```bash
cargo run                           # debug build, launches the GUI
cargo run -- --config path.toml     # custom config location
cargo build --release
cargo test                          # all tests, incl. integration
```

On first launch:
1. A default `config.toml` is written next to the binary (or in CWD).
2. With `[capture] mode = "mitm"` (default), the proxy generates a
   self-signed root CA at `./ca/akagi-ca.{cer,crt,pem,der}`. Trust it in
   your OS / browser store before pointing Majsoul at the proxy.
3. With `[capture] mode = "chromium"`, Akagi launches a controlled
   Chromium-family browser (auto-detected: Chrome / Edge / Brave /
   Chromium) with its own profile under `<user_config_root>/chrome-profile`
   and intercepts WebSocket frames via the Chrome DevTools Protocol — no
   proxy / CA setup needed. Settings → Capture has the toggle and a
   Detect button to pick the browser executable.
4. The first bot spawn runs `uv sync` (slow). Subsequent spawns are fast —
   the sync is gated by an mtime+size stamp at `mjai_bot/<name>/.akagi/synced.stamp`.

Default proxy bind: `127.0.0.1:23410`. Health probe: `GET /ping → pong`.

---

## Configuration

`config.toml`:

```toml
[general]
language = "en"

[logging]
dir       = "./logs"
level     = "info"
all_level = "warn"

[platform]
kind = "Majsoul"

[proxy]
enabled = true
addr    = "127.0.0.1:23410"
ca_dir  = "./ca"

[capture]
mode = "mitm"               # or "chromium"

[capture.chromium]
executable    = ""                              # blank = auto-detect
user_data_dir = ""                              # blank = <user_config_root>/chrome-profile
start_url     = "https://game.maj-soul.com/1/"
cft_channel   = "stable"                        # Chrome-for-Testing fallback (Phase 2)
force_cft     = false
extra_args    = []

[bot]
enabled   = true
active_4p = "mortal"        # bot used in 4-player (yonma) games
active_3p = "mortal3p"      # bot used in 3-player (sanma) games; empty = none
auto_sync = true
dir       = "./mjai_bot"
```

Edit live via the `update_config` Tauri command. Capture-affecting
fields (`capture.mode`, `capture.chromium.*`, `proxy.*`) automatically
restart the active backend on save — no app relaunch needed.
`bot.active_4p` / `bot.active_3p` swap on the next `start_game`
(per-mode based on the table's `num_players`). Pre-3p config files with
a single `active = "..."` key are auto-migrated into `active_4p` on load.

---

## Mjai Bot

Each bot runs as a **separate OS subprocess** spawned by Akagi. The host
process talks to it strictly over stdin / stdout JSONL — no in-process
linking, no shared address space, no FFI. This is an intentional license
boundary: an AGPL-licensed bot (e.g. Mortal, which links libriichi) stays
inside its own process, so dropping it under `mjai_bot/<name>/` does **not**
make Akagi a derived work of the bot.

Because of this isolation, bots are not bundled in this repo and must be
obtained separately. Only the rule-based `mjai_bot/example/` ships in tree
as a known-good reference. For NN bots like Mortal, fetch them via the
`install_bot_from_github` IPC command (or the matching frontend button),
which downloads the latest release zip from a public GitHub repo and
drops it into `mjai_bot/<name>/`. Manual placement still works for repos
without published releases.

### Bot layout

```
mjai_bot/<name>/
├── bot.py           # JSONL stdin → JSONL stdout
├── pyproject.toml   # requires-python = ">=3.12"
└── README.md
```

`bot.py` reads one JSON array of `MjaiEvent`s per line and writes one
mjai action object per line (`{"type":"none"}` when no action is owed).
Akagi pumps stderr into `tracing` with `bot=<name>`. See
`src/bot/README.md` for the full contract and `mjai_bot/example/` for a
working reference.

---

## Game History

Every cleanly-ended match (i.e. one that produced an `end_game` mjai event)
is persisted under `<config_root>/history/`:

```
<config_root>/history/
├── index.jsonl              # one JSON-encoded GameRecord per line
└── games/
    └── <ulid>.mjai.jsonl    # full event stream copy
```

Mid-game disconnects leave an unfinalised buffer that is silently dropped — only
complete games make it to disk. The history store is independent of the
session log dir (`<log_dir>/majsoul/*.mjai.jsonl`), which keeps its bridge-debug
behaviour untouched.

The frontend's **History** tab (sidebar) shows:

- **Rank pie chart** — 1st / 2nd / 3rd / 4th distribution (3 slices for sanma).
- **Cumulative PT line chart** — running total under the chosen rule:
  - **Majsoul**: pick `場次` (銅 / 銀 / 金 / 玉 / 王座) and `段位` (初心 1 星 → 魂天).
    PT = `(終局点 − 25_000) / 1000 + 馬點 + 段位分`. 3p uses 35_000 starting
    score and the sanma uma/dan tables.
  - **Tenhou**: pick `段位` (新人 → 天鳳位 across 21 ranks). PT comes
    straight from a `[段位][東/南][rank]` cell.
  - **Custom**: edit the uma + dan-bonus arrays for 4p and 3p directly.
  Switching rule / dan re-renders the chart immediately — no backend
  round-trip.
- **Detailed stats** — win rate, deal-in rate, riichi rate, fuuro rate,
  ryukyoku rate, average winning / deal-in points, average winning turn,
  yakuman / nagashi-mangan counts; mirrors `libriichi/src/stat.rs`.
- **Game list** — filterable by platform / players / east-or-south / date.
  Click a row to see the final standings and per-game stats; the trash
  icon deletes both the index entry and the per-game `.mjai.jsonl` copy.

PT-rule and filter selections are persisted to `localStorage`; the records
themselves come from the backend on bridge boot and stay current via the
`history-recorded` Tauri event.

See `src/history/README.md` for the module-extension guide (adding a new
platform / stat field / filter dimension).

---

## TODO

- [ ] Complete frontend
- [x] 3-player mahjong (sanma) — full pipeline: bridge / tracker / snapshot / analysis / FE / per-mode bot routing. See `reference/reference_mjai_3p.md` for the wire format.
- [ ] Other platforms (Tenhou / RiichiCity)
- [x] Download mjai bot from GitHub repo link (per-bot release URL → auto-fetch into `mjai_bot/<name>/`)
- [x] Game history persistence + frontend tab (rank pie / cumulative PT / stats / game list)

---

## Frontend

Static HTML / CSS / JS under `frontend/`, served by Tauri. The Tauri
command + event surface is documented in `frontend/README.md`
(when present locally) — schemas are in `src/schema/`, `src/ipc/`,
`src/analysis/result.rs`, `src/game_state/snapshot.rs`,
`src/game_state/mahgen_view.rs`.

The board uses [mahgen](https://github.com/eric200203/mahgen)'s `<mah-gen>`
custom element. The backend pre-encodes hands / melds / rivers / dora as
mahgen DSL strings (`get_mahgen_view` command) so the frontend only has to
plug strings into the element.

---

## Reference materials

| Source | Used in | What for |
|---|---|---|
| [mjai JSONL spec (Gimite)](https://gimite.net/pukiwiki/index.php?Mjai%20%E9%BA%BB%E9%9B%80AI%E5%AF%BE%E6%88%A6%E3%82%B5%E3%83%BC%E3%83%90) | `src/schema/mjai/` | `MjaiEvent` enum + bot wire contract — 15 event types, tile-string format, state-machine rules. |
| [`EndlessCheng/mahjong-helper`](https://github.com/EndlessCheng/mahjong-helper) (Go analysis CLI) | `src/analysis/` | Direct Rust port of `util/` — shanten, waits, agari-rate, tenpai-rate, risk model, discard search. |
| [`Xerxes-2/MajsoulMax-rs`](https://github.com/Xerxes-2/MajsoulMax-rs) (Rust MITM proxy, **GPL-3.0**) | `src/proxy/handler.rs`, `src/bridge/majsoul/parser.rs`, `src/bridge/majsoul/proto/liqi.proto` | Reference for the 5-layer Majsoul WS wire format (type byte → Wrapper → inner message → action protobuf). **Format only — no code copied.** |
| [`smly/RiichiEnv`](https://github.com/smly/RiichiEnv) (Rust RL env w/ Python bindings) | `Cargo.toml` (`riichienv-core` dep), `src/analysis/`, `src/game_state/` | Tile / hand / shanten / yaku / score primitives + game-state model. The analysis engine and game tracker are built on this. |
| [`eric200203/mahgen`](https://github.com/eric200203/mahgen) (mahjong-tile rendering DSL) | `src/game_state/mahgen_view.rs`, frontend `<mah-gen>` | DSL syntax for pre-encoding hand / meld / river strings backend-side. |
| [`smly/mjai.app`](https://github.com/smly/mjai.app) (mahjong AI competition platform) | `mjai_bot/`, `src/bot/` | Bot subprocess convention — JSONL stdin/stdout, argv `python bot.py <player_id>`, `AKAGI_PLAYER_ID` env, end-of-batch flush points. |
| [`shinkuan/Akagi`](https://github.com/shinkuan/Akagi) (original Akagi, Python) | Architecture / behaviour parity | The original feature set we are reproducing: MITM proxy, mjai bridge, pluggable bots, recommendation HUD. |

---

## License

Akagi v3 is licensed under the [Apache License 2.0](./LICENSE.txt).
Copyright 2026 Shinkuan.

Third-party attributions live in [`NOTICE`](./NOTICE) — read it alongside the
license. Per Apache-2.0 §4(d), redistributions must include both files.

Bundled / linked sources:

- **mahjong-helper** (MIT) — `src/analysis/` is a Rust port of `util/`.
- **riichienv-core** / RiichiEnv (Apache-2.0) — Cargo dependency.
- **mahgen** (MIT) — DSL + `<mah-gen>` custom element.

Reference-only (no code copied, listed in `NOTICE` for credit):

- **MajsoulMax-rs** (GPL-3.0) — Majsoul WS wire format reference only.
- **mjai spec** (Gimite) — bot wire contract.
- **mjai.app** — bot subprocess convention.

### Bots and AGPL

Bots run in a separate OS process and talk to Akagi over JSONL stdin/stdout.
This is an intentional license boundary: AGPL-licensed bots (e.g. Mortal,
which links libriichi) stay inside their own process, so dropping one under
`mjai_bot/<name>/` does **not** make Akagi a derived work of the bot. See
`src/bot/README.md` for the contract.
