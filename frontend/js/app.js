// Akagi V3 frontend — Tauri IPC bridge + renderer.

import { LANGS, t, getPack, setLang, getLang, initFromStorage, onLangChange, applyDom } from "./i18n.js";

// =============== Tauri bridge ===============
const TAURI = window.__TAURI__;
const HAS_TAURI = !!TAURI;

async function invoke(cmd, args) {
  if (!HAS_TAURI) throw new Error("not in tauri");
  return await TAURI.core.invoke(cmd, args);
}

async function listenEvent(name, cb) {
  if (!HAS_TAURI) return () => {};
  return await TAURI.event.listen(name, cb);
}

// =============== App state ===============
const state = {
  config: null,
  botStatus: { state: "idle" },
  proxyStatus: { state: "stopped" },
  logDir: "",
  game: null,            // GameStateSnapshot
  view: null,            // MahgenView
  analysis: null,        // AnalysisResult
  events: [],            // recent mjai events
  responses: [],         // recent bot responses
  notifications: [],     // toast log
};

// =============== Helpers ===============
const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => Array.from(root.querySelectorAll(sel));

function fmtScore(n) {
  return new Intl.NumberFormat("en-US").format(n ?? 0);
}

function fmtTime(d = new Date()) {
  const h = String(d.getHours()).padStart(2, "0");
  const m = String(d.getMinutes()).padStart(2, "0");
  const s = String(d.getSeconds()).padStart(2, "0");
  return `${h}:${m}:${s}`;
}

function pct(v, digits = 1) {
  if (v == null || isNaN(v)) return "—";
  return `${Number(v).toFixed(digits)}%`;
}

function riskClass(v) {
  if (v == null) return "";
  if (v >= 20) return "risk-high";
  if (v >= 10) return "risk-mid";
  return "risk-low";
}

// Relative position label vs our seat: 0=self, 1=shimocha (next), 2=toimen (across), 3=kamicha (prev).
function relativeKind(seat, ourSeat) {
  if (ourSeat == null) return "self";
  const d = (seat - ourSeat + 4) % 4;
  return d === 0 ? "self" : d === 1 ? "shimocha" : d === 2 ? "toimen" : "kamicha";
}

// mjai bakaze ("E"/"S"/"W"/"N") + kyoku → display
function kyokuLabel(g) {
  if (!g) return t("kyoku_fmt", { bakaze: "—", kyoku: "—" });
  return t("kyoku_fmt", { bakaze: getPack().bakaze[g.bakaze] ?? g.bakaze, kyoku: g.kyoku });
}

// =============== mahgen helper ===============
// MahgenElement (reference/mahgen/src/MahgenElement.ts) renders a single <img>
// into an open shadow root. There is no exposed CSS hook, so we size that
// inner <img> directly from JS. Each mah-gen element is registered with a
// "kind" so we know which container drives its target height, and a global
// ResizeObserver re-applies sizes when the layout changes.

// Per-context sizing.
// `mode: "river"`  -> assume mahgen lays at most 6 tiles per row (see
//                     reference/mahgen/src/JimpWorker.ts). Pick a uniform
//                     scale so that 6 tiles span exactly the container
//                     width — additional rows then grow downward but each
//                     tile keeps the same on-screen size, which is what the
//                     player expects from a real river.
// `mode: "fit"`    -> aspect-fit a single-row image (e.g. self hand) to the
//                     container width, capped by `max`.
// `mode: "linear"` -> scale a target tile height by container width. Used
//                     when multiple mahgens share a row (melds), or when
//                     the container is much wider than the image (dora, rec).
const SIZE_CTX = {
  river: { mode: "river", maxScale: 0.65, minScale: 0.18 },
  hand:  { mode: "fit",   min: 44, max: 100 },
  melds: { mode: "linear", base: 34, ref: 230, min: 22, max: 56 },
  // Dora sits in the top bar pill — its container is content-sized, so
  // a container-based formula self-feeds. Use a fixed pixel height instead.
  dora:  { mode: "fixed",  base: 30 },
  rec:   { mode: "linear", base: 38, ref: 340, min: 28, max: 56 },
};

// Native tile dimensions from reference/mahgen/res — normal portrait tile is
// 70x100. River-mode lays 6 wide, so a full row is 6 × 70 = 420 px.
const RIVER_FULL_ROW_W = 420;

const mahgenRegistry = new Map(); // mah-gen element -> { kind, container }

function applyMahgenSize(el) {
  const entry = mahgenRegistry.get(el);
  if (!entry) return;
  if (!el.isConnected) {
    // Either (a) the element was just constructed and the parent panel hasn't
    // been appended to the DOM yet — registerMahgen runs inside buildPanel,
    // before root.appendChild(panel) — or (b) the element was removed (e.g.
    // rec list innerHTML wipe). Retry a few frames; if still detached, drop.
    entry._retries = (entry._retries || 0) + 1;
    if (entry._retries > 6) { mahgenRegistry.delete(el); return; }
    requestAnimationFrame(() => applyMahgenSize(el));
    return;
  }
  entry._retries = 0;
  const root = el.shadowRoot;
  if (!root) {
    // The custom element hasn't been upgraded yet (mahgen UMD not loaded).
    requestAnimationFrame(() => applyMahgenSize(el));
    return;
  }
  const img = root.querySelector("img");
  if (!img) {
    requestAnimationFrame(() => applyMahgenSize(el));
    return;
  }

  // Re-apply each time the image finishes loading so we can read
  // naturalWidth/Height as soon as they are known. Without this, the first
  // paint uses the fallback size and never gets corrected.
  if (!img._akagiOnLoad) {
    img._akagiOnLoad = true;
    img.addEventListener("load", () => applyMahgenSize(el));
  }

  // Hide the host entirely when there is no sequence to render — otherwise
  // an empty river leaves a sized blank rectangle in the layout.
  const seq = el.getAttribute("data-seq");
  if (!seq) {
    el.style.display = "none";
    return;
  }
  el.style.display = "";

  const cfg = SIZE_CTX[entry.kind];
  const cw = entry.container?.clientWidth || cfg.ref || 200;
  const nw = img.naturalWidth;
  const nh = img.naturalHeight;
  const aspectKnown = nw > 0 && nh > 0;

  // We set both width AND height on the host AND on the inner <img>.
  // `max-width: 100%` on the img inside the shadow root is circular (the
  // containing block resolves to the host's content box, and the host is
  // inline-block so it sizes to its content), so explicit pixel dims are
  // the only reliable handle.
  let w, h;
  if (cfg.mode === "river") {
    // Pick a uniform scale so 6 tiles == container width. More tiles wrap to
    // additional rows but stay at the same on-screen size.
    if (aspectKnown) {
      let scale = cw / RIVER_FULL_ROW_W;
      if (cfg.maxScale) scale = Math.min(scale, cfg.maxScale);
      if (cfg.minScale) scale = Math.max(scale, cfg.minScale);
      w = nw * scale;
      h = nh * scale;
    } else {
      w = cw;
      h = (cw / RIVER_FULL_ROW_W) * 100; // single-row fallback before load
    }
  } else if (cfg.mode === "fit") {
    if (aspectKnown) {
      w = cw;
      h = (cw * nh) / nw;
      if (cfg.max && h > cfg.max) { h = cfg.max; w = (h * nw) / nh; }
      if (cfg.min && h < cfg.min) { h = cfg.min; w = (h * nw) / nh; }
    } else {
      w = cw;
      h = cfg.max;
    }
  } else if (cfg.mode === "fixed") {
    h = cfg.base;
    w = aspectKnown ? (h * nw) / nh : cfg.base;
  } else {
    // linear
    h = cfg.base * (cw / cfg.ref);
    h = Math.max(cfg.min, Math.min(cfg.max, h));
    w = aspectKnown ? (h * nw) / nh : cfg.base;
  }

  el.style.width = w + "px";
  el.style.height = h + "px";
  el.style.flex = "0 0 auto";
  img.style.width = w + "px";
  img.style.height = h + "px";
  img.style.objectFit = "contain";
  img.style.display = "block";
}

// One shared observer — when any registered container resizes (e.g. the rail
// drag changes the players column width), re-apply size for every mahgen in
// that container.
const _mahgenRO = new ResizeObserver((entries) => {
  const containers = new Set(entries.map((e) => e.target));
  for (const [el, ent] of mahgenRegistry) {
    if (containers.has(ent.container)) applyMahgenSize(el);
  }
});

function registerMahgen(el, kind, container) {
  mahgenRegistry.set(el, { kind, container });
  if (container) _mahgenRO.observe(container);
  applyMahgenSize(el);
}

function unregisterMahgen(el) {
  mahgenRegistry.delete(el);
}

let _mahgenResizeRaf = 0;
function scheduleMahgenResize() {
  if (_mahgenResizeRaf) return;
  _mahgenResizeRaf = requestAnimationFrame(() => {
    _mahgenResizeRaf = 0;
    for (const el of mahgenRegistry.keys()) applyMahgenSize(el);
  });
}
window.addEventListener("resize", scheduleMahgenResize);

// Returns a fragment containing a <mah-gen> tag (or fallback if WC unavailable yet).
function mahgenEl(seq, riverMode = false) {
  const el = document.createElement("mah-gen");
  if (seq) el.setAttribute("data-seq", seq);
  if (riverMode) el.setAttribute("data-river-mode", "");
  return el;
}

// Update <mah-gen> data-seq in place with a smooth opacity crossfade so the WC
// can swap its internal tiles without the panel flickering.
function setMahgenSeq(el, seq) {
  const next = seq ?? "";
  if ((el.getAttribute("data-seq") ?? "") === next) return;
  el.style.opacity = "0";
  // Two RAFs: let the opacity:0 paint, then swap content, then fade back in.
  requestAnimationFrame(() => {
    el.setAttribute("data-seq", next);
    requestAnimationFrame(() => {
      el.style.opacity = "1";
      // Ensure inner <img> styling persists after the WC replaces src.
      applyMahgenSize(el);
    });
  });
}

// Convert mjai tile array → mahgen DSL (used for fallback / dora helper).
function mjaiToMahgen(tiles) {
  if (!tiles || !tiles.length) return "";
  let backs = 0;
  const m = [], p = [], s = [], z = [];
  const zMap = { E: 1, S: 2, W: 3, N: 4, P: 5, F: 6, C: 7 };
  for (const tile of tiles) {
    if (!tile || tile === "?") { backs++; continue; }
    if (zMap[tile]) { z.push(zMap[tile]); continue; }
    const isRed = tile.endsWith("r");
    const suit = isRed ? tile[tile.length - 2] : tile[tile.length - 1];
    const num = isRed ? 0 : parseInt(tile[0], 10);
    if (suit === "m") m.push(num);
    else if (suit === "p") p.push(num);
    else if (suit === "s") s.push(num);
  }
  const sortKey = (a, b) => (a === 0 ? 5.5 : a) - (b === 0 ? 5.5 : b);
  m.sort(sortKey); p.sort(sortKey); s.sort(sortKey); z.sort((a, b) => a - b);
  let out = "";
  if (m.length) out += m.join("") + "m";
  if (p.length) out += p.join("") + "p";
  if (s.length) out += s.join("") + "s";
  if (z.length) out += z.join("") + "z";
  if (backs > 0) out += "0".repeat(backs) + "z";
  return out;
}

// =============== Render: players ===============
// Cached panel structures keyed by seat — we update fields in place each render
// so <mah-gen> elements stay mounted (avoids the WC re-creation flicker).
const panelCache = new Map();
let selfHandCache = null;

function buildPanel() {
  const panel = document.createElement("article");
  panel.className = "player";

  const head = document.createElement("div");
  head.className = "player-head";
  const labelEl = document.createElement("div");
  labelEl.className = "player-label cjk";
  const seatSpan = document.createElement("span");
  seatSpan.className = "seat";
  const labelText = document.createTextNode("");
  labelEl.append(labelText, " ", seatSpan);
  const windChip = document.createElement("div");
  windChip.className = "wind-chip cjk";
  head.append(labelEl, windChip);

  const scoreRow = document.createElement("div");
  scoreRow.className = "score-row";
  const scoreNum = document.createElement("span");
  scoreNum.className = "score-num";
  const scoreStick = document.createElement("span");
  scoreStick.className = "score-stick";
  scoreStick.innerHTML = `<span class="stick-bar"></span><span class="stick-val"></span>`;
  scoreRow.append(scoreNum, scoreStick);

  const meldsTitle = document.createElement("div");
  meldsTitle.className = "player-section-h cjk";
  const meldsRow = document.createElement("div");
  meldsRow.className = "tile-row melds-row";

  const riverTitle = document.createElement("div");
  riverTitle.className = "player-section-h cjk";
  const riverRow = document.createElement("div");
  riverRow.className = "tile-row river-row";
  const riverMahgen = document.createElement("mah-gen");
  riverMahgen.setAttribute("data-river-mode", "");
  riverRow.appendChild(riverMahgen);

  panel.append(head, scoreRow, meldsTitle, meldsRow, riverTitle, riverRow);

  // Register the river mahgen with its own row as the sizing container —
  // that's the element whose clientWidth bounds the rendered tiles.
  registerMahgen(riverMahgen, "river", riverRow);

  return {
    panel, labelText, seatSpan, windChip, scoreNum, scoreStick,
    meldsTitle, meldsRow, riverTitle, riverMahgen,
  };
}

function updateMeldsRow(row, melds, panel) {
  // Remove any "—" empty marker
  const empty = row.querySelector(".meld-empty");
  if (melds && melds.length) {
    if (empty) empty.remove();
    // Drop extra mahgen children
    const mahgens = row.querySelectorAll("mah-gen");
    for (let i = mahgens.length - 1; i >= melds.length; i--) {
      unregisterMahgen(mahgens[i]);
      mahgens[i].remove();
    }
    // Update / create
    for (let i = 0; i < melds.length; i++) {
      let el = row.querySelectorAll("mah-gen")[i];
      if (!el) {
        el = document.createElement("mah-gen");
        row.appendChild(el);
        registerMahgen(el, "melds", row);
      }
      setMahgenSeq(el, melds[i]);
    }
  } else {
    // Clear mahgens, show "—"
    row.querySelectorAll("mah-gen").forEach((m) => {
      unregisterMahgen(m);
      m.remove();
    });
    if (!empty) {
      const span = document.createElement("span");
      span.className = "meld-empty";
      span.style.color = "var(--t-4)";
      span.style.fontSize = "11px";
      span.textContent = "—";
      row.appendChild(span);
    }
  }
}

function renderPlayers() {
  const root = $("#players");
  const game = state.game;
  const view = state.view;
  if (!game || !view) {
    root.innerHTML = `<div class="rail-card empty" style="grid-column: 1/-1">No game loaded.</div>`;
    panelCache.clear();
    return;
  }
  // First call after an empty state: clear placeholder
  if (root.querySelector(".rail-card.empty")) root.innerHTML = "";

  const ourSeat = game.our_seat;
  const order = ourSeat == null
    ? [0, 1, 2, 3]
    : [ourSeat, (ourSeat + 3) % 4, (ourSeat + 2) % 4, (ourSeat + 1) % 4];

  const pack = getPack();

  for (const seat of order) {
    let p = panelCache.get(seat);
    if (!p) {
      p = buildPanel();
      panelCache.set(seat, p);
    }
    const ps = game.players[seat];
    const pv = view.players[seat];
    const rel = relativeKind(seat, ourSeat);
    const isSelf = rel === "self";
    const isCurrent = game.current_player === seat;

    p.panel.className = "player"
      + (isSelf ? " is-self" : "")
      + (isCurrent ? " is-current" : "");

    p.labelText.nodeValue = pack.relative[rel] + " ";
    p.seatSpan.textContent = `${pack.seatsLatin[seat]} (${seat})`;
    p.windChip.textContent = pack.seats[(seat - game.oya + 4) % 4];

    p.scoreNum.textContent = fmtScore(ps.score);
    p.scoreStick.querySelector(".stick-val").textContent =
      game.kyotaku > 0 ? game.kyotaku * 1000 : 1000;

    p.meldsTitle.textContent = pack.dict["section.melds"] ?? "副露";
    p.riverTitle.textContent = pack.dict["section.river"] ?? "牌河";

    updateMeldsRow(p.meldsRow, pv.melds || [], p.panel);
    setMahgenSeq(p.riverMahgen, pv.river || "");

    // Re-append to enforce DOM order matching `order`.
    root.appendChild(p.panel);
  }

  // Render the self-hand strip below the four columns.
  renderSelfHand(ourSeat, game, view, pack);
}

function renderSelfHand(ourSeat, game, view, pack) {
  const root = $("#selfHand");
  if (ourSeat == null || !view?.players?.[ourSeat]) {
    root.innerHTML = "";
    selfHandCache = null;
    return;
  }
  if (!selfHandCache) {
    root.innerHTML = "";
    const tiles = document.createElement("div");
    tiles.className = "self-hand-tiles";
    const mahgen = document.createElement("mah-gen");
    tiles.appendChild(mahgen);
    root.append(tiles);
    registerMahgen(mahgen, "hand", root);
    selfHandCache = { tiles, mahgen };
  }
  setMahgenSeq(selfHandCache.mahgen, view.players[ourSeat].hand || "");
}

// =============== Render: dora ===============
let doraMahgen = null;
function renderDora() {
  const wrap = $("#doraTiles");
  if (!doraMahgen || !wrap.contains(doraMahgen)) {
    wrap.innerHTML = "";
    doraMahgen = document.createElement("mah-gen");
    wrap.appendChild(doraMahgen);
    registerMahgen(doraMahgen, "dora", wrap);
  }
  const seq = state.view?.dora_indicators
    ?? (state.game?.dora_markers?.length ? mjaiToMahgen(state.game.dora_markers) : "");
  setMahgenSeq(doraMahgen, seq);
}

// =============== Render: top bar ===============
function renderTopbar() {
  const g = state.game;
  $("#kyokuTitle").textContent = kyokuLabel(g);
  $("#mHonba").textContent = g?.honba ?? 0;
  $("#mKyotaku").textContent = g?.kyotaku ?? 0;
  $("#mTurn").textContent = g?.turn_count ?? 0;
  $("#fTurn").textContent = state.analysis?.turn ?? g?.turn_count ?? 0;

  const pack = getPack();
  if (g) {
    const cp = g.current_player;
    $("#mCurrent").textContent = `${pack.seatsLatin[cp]} (${cp})`;
    $("#mPhase").textContent = g.phase ?? "—";
  } else {
    $("#mCurrent").textContent = "—";
    $("#mPhase").textContent = "—";
  }
}

// =============== Render: rail (recommendations / risk / opponents) ===============
function renderRail() {
  const a = state.analysis;
  const pack = getPack();

  // Top 3 maintain
  const recList = $("#recList");
  // Drop registry entries for the mahgens we're about to wipe.
  recList.querySelectorAll("mah-gen").forEach((m) => unregisterMahgen(m));
  recList.innerHTML = "";
  if (a?.hand14?.maintain?.length) {
    const top3 = a.hand14.maintain.slice(0, 3);
    top3.forEach((c, i) => {
      const dealIn = state.analysis.mixed_risk?.[mjaiTileIdx(c.discard)] ?? 0;
      const li = document.createElement("li");
      li.className = "rec";
      li.innerHTML = `
        <div class="rec-rank">${i + 1}</div>
        <div class="rec-tile"></div>
        <div class="rec-stat">
          <span class="rec-k">${pack.dict["score"]}</span>
          <span class="rec-v">${(c.result.mixed_waits_score ?? 0).toFixed(0)}</span>
        </div>
        <div class="rec-stat">
          <span class="rec-k">${pack.dict["ev_round"]}</span>
          <span class="rec-v ${(c.result.mixed_round_point ?? 0) >= 0 ? "good" : "bad"}">${
            (c.result.mixed_round_point ?? 0) >= 0 ? "+" : ""}${(c.result.mixed_round_point ?? 0).toFixed(2)}</span>
        </div>
        <div class="rec-stat">
          <span class="rec-k">${pack.dict["deal_in_risk"]}</span>
          <span class="rec-v ${riskClass(dealIn)}">${pct(dealIn)}</span>
        </div>
      `;
      const bar = document.createElement("div");
      bar.className = "rec-bar";
      const score = c.result.mixed_waits_score ?? 0;
      bar.style.height = `${Math.min(56, 24 + score / 30)}px`;
      li.appendChild(bar);
      const recTileWrap = li.querySelector(".rec-tile");
      const recMahgen = mahgenEl(mjaiToMahgen([c.discard]));
      recTileWrap.appendChild(recMahgen);
      // Use the whole rec list as the size container so rail width drives
      // the tile size (rec-tile column is auto-sized and tiny).
      registerMahgen(recMahgen, "rec", recList);
      recList.appendChild(li);
    });
    $("#shantenPill").textContent = `${pack.dict["shanten"]} ${a.shanten}`;
  } else {
    recList.innerHTML = `<li class="rail-card empty" style="border:none;padding:18px 0;">${
      pack.dict["rail.analysis_empty"]
    }</li>`;
    $("#shantenPill").textContent = `${pack.dict["shanten"]} —`;
  }

  // Risk chart (34 bars)
  const chart = $("#riskChart");
  chart.innerHTML = "";
  const grid = document.createElement("div");
  grid.className = "risk-grid";
  grid.innerHTML = `<span>30%</span><span>15%</span><span>0%</span>`;
  chart.appendChild(grid);
  const risks = a?.mixed_risk ?? new Array(34).fill(0);
  const max = 30;
  for (let i = 0; i < 34; i++) {
    const bar = document.createElement("div");
    bar.className = "risk-bar";
    const v = Math.min(max, risks[i] ?? 0);
    bar.style.height = `${(v / max) * 100}%`;
    chart.appendChild(bar);
  }
  const axis = document.createElement("div");
  axis.className = "risk-axis";
  axis.innerHTML = `<span>1m</span><span>9m</span><span>1p</span><span>9p</span><span>1s</span><span>9s</span><span>E</span><span>S</span><span>W</span><span>N</span><span>P</span><span>F</span><span>C</span>`;
  chart.appendChild(axis);

  // Opponents table
  const ob = $("#oppBody");
  ob.innerHTML = "";
  const opps = a?.opponents ?? [];
  for (const o of opps) {
    const maxRisk = Math.max(...(o.risk ?? [0]));
    const tr = document.createElement("tr");
    tr.innerHTML = `
      <td class="cjk">${pack.seatsLatin[o.seat]} (${o.seat})</td>
      <td class="tdval">${pct(o.tenpai_rate)}</td>
      <td class="${o.is_riichi ? "opp-riichi-yes" : "opp-riichi-no"}">${o.is_riichi ? "✓" : "—"}</td>
      <td class="tdval ${riskClass(maxRisk)}">${pct(maxRisk)}</td>
    `;
    ob.appendChild(tr);
  }
  if (!opps.length) {
    ob.innerHTML = `<tr><td colspan="4" style="color:var(--t-4);text-align:center;padding:18px 0;">—</td></tr>`;
  }

  $("#fState").textContent = a?.state ?? "—";
  $("#fUpdated").textContent = fmtTime();
}

function mjaiTileIdx(mjai) {
  if (!mjai) return 0;
  const zMap = { E: 27, S: 28, W: 29, N: 30, P: 31, F: 32, C: 33 };
  if (zMap[mjai] != null) return zMap[mjai];
  const isRed = mjai.endsWith("r");
  const suit = isRed ? mjai[mjai.length - 2] : mjai[mjai.length - 1];
  const num = isRed ? 5 : parseInt(mjai[0], 10);
  const base = suit === "m" ? 0 : suit === "p" ? 9 : 18;
  return base + (num - 1);
}

// =============== Render: side cards (bot/proxy/log) ===============
function renderSide() {
  const bs = state.botStatus;
  const row = $("#botStatusRow .dot");
  $("#botState").textContent = bs.state ?? "—";
  $("#botName").textContent = bs.bot ?? state.config?.bot?.active ?? "—";
  $("#botActor").textContent = bs.actor_id != null ? `actor_id: ${bs.actor_id}` : "";
  row.className = "dot " + (bs.state === "ready" ? "dot-ok" : bs.state === "error" ? "dot-err" : bs.state === "loading" ? "dot-warn" : "");

  const ps = state.proxyStatus;
  $("#proxyState").textContent = ps.state ?? "—";
  $("#proxyAddr").textContent = ps.addr ?? "";
  $("#proxyStatusRow .dot").className = "dot " + (ps.state === "running" ? "dot-ok" : ps.state === "error" ? "dot-err" : ps.state === "starting" ? "dot-warn" : "");

  if (state.logDir) {
    const parts = state.logDir.split(/[\\/]/);
    $("#logName").textContent = parts[parts.length - 1] || state.logDir;
    $("#logPath").textContent = state.logDir;
  }
}

// =============== Render: events / notifications / responses ===============
function renderEvents() {
  const list = $("#eventList");
  list.innerHTML = "";
  const recent = state.events.slice(-12).reverse();
  const pack = getPack();
  for (const e of recent) {
    const row = document.createElement("li");
    row.className = "event-row";
    const time = e._time ?? fmtTime();
    const seatLabel = (e.actor != null) ? `${pack.seatsLatin[e.actor][0]}(${e.actor})` : "";
    const pai = e.pai ?? (e.consumed ? e.consumed.join(",") : "");
    let meta = "";
    if (e.type === "dahai") meta = e.tsumogiri ? pack.dict["tag.tsumogiri"] : pack.dict["tag.tedashi"];
    if (e.type === "pon" || e.type === "chi" || e.type === "daiminkan") {
      meta = `from ${pack.seatsLatin[e.target]?.[0] ?? "?"}(${e.target})`;
    }
    if (e.type === "reach") meta = pack.dict["tag.riichi"];
    row.innerHTML = `
      <span class="dot dot-ok"></span>
      <span class="ev-time">${time}</span>
      <span class="ev-kind">${e.type}</span>
      <span class="ev-actor">${seatLabel} ${pai}</span>
      <span class="ev-meta">${meta}</span>
    `;
    list.appendChild(row);
  }
  if (!recent.length) {
    list.innerHTML = `<li style="padding:12px 14px;color:var(--t-4);font-size:11px;">—</li>`;
  }
}

function renderNotifications() {
  const list = $("#notifList");
  list.innerHTML = "";
  const recent = state.notifications.slice(-8).reverse();
  for (const n of recent) {
    const row = document.createElement("li");
    row.className = "notif-row";
    const ico = (n.level === "success") ? "✓"
      : (n.level === "error") ? "!"
      : (n.level === "warn") ? "!" : "i";
    row.innerHTML = `
      <span class="notif-icon ${n.level}">${ico}</span>
      <div class="notif-body">
        <div class="notif-title">${escapeHtml(n.title)}</div>
        ${n.body ? `<div class="notif-sub">${escapeHtml(n.body)}</div>` : ""}
      </div>
      <span class="notif-time">${n._time ?? fmtTime()}</span>
    `;
    list.appendChild(row);
  }
  if (!recent.length) {
    list.innerHTML = `<li style="padding:12px 14px;color:var(--t-4);font-size:11px;">—</li>`;
  }
}

function renderResponses() {
  const list = $("#respList");
  list.innerHTML = "";
  const pack = getPack();
  const recent = state.responses.slice(-8).reverse();
  for (const r of recent) {
    const row = document.createElement("li");
    row.className = "resp-row";
    const time = r._time ?? fmtTime();
    const pai = r.pai ?? "";
    let meta = "";
    if (r.type === "dahai") meta = r.tsumogiri ? pack.dict["tag.tsumogiri"] : pack.dict["tag.tedashi"];
    else if (r.type === "pon" || r.type === "chi" || r.type === "daiminkan") {
      meta = `from ${pack.seatsLatin[r.target]?.[0] ?? "?"}(${r.target})`;
    }
    row.innerHTML = `
      <span class="ev-time">${time}</span>
      <span class="ev-kind">${r.type}</span>
      <span class="ev-pai">${pai}</span>
      <span class="ev-meta">${meta}</span>
    `;
    list.appendChild(row);
  }
  if (!recent.length) {
    list.innerHTML = `<li style="padding:12px 14px;color:var(--t-4);font-size:11px;">—</li>`;
  }
}

function escapeHtml(s) {
  return String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
}

// =============== Full re-render ===============
function renderAll() {
  renderTopbar();
  renderPlayers();
  renderDora();
  renderRail();
  renderSide();
  renderEvents();
  renderNotifications();
  renderResponses();
}

// =============== Wiring ===============
function wireNav() {
  for (const item of $$(".nav-item")) {
    item.addEventListener("click", () => {
      $$(".nav-item").forEach((n) => n.classList.remove("active"));
      item.classList.add("active");
    });
  }
}

function wireTabs() {
  for (const tab of $$(".tab")) {
    tab.addEventListener("click", () => {
      const name = tab.dataset.tab;
      $$(".tab").forEach((t) => t.classList.toggle("active", t === tab));
      $$(".tab-pane").forEach((p) => p.classList.toggle("active", p.dataset.pane === name));
    });
  }
}

function wireLang() {
  const sel = $("#langSelect");
  sel.value = getLang();
  sel.addEventListener("change", (e) => setLang(e.target.value));
  onLangChange(() => renderAll());
}

// =============== Resize / collapse: bottom bar + rail ===============
const LS_RAIL_W = "akagi.railW";
const LS_BOTTOM_H = "akagi.bottomH";
const LS_BOTTOM_COLLAPSED = "akagi.bottomCollapsed";

function clamp(v, lo, hi) { return Math.max(lo, Math.min(hi, v)); }

function wireRailResize() {
  const board = document.querySelector(".board");
  const rail = document.querySelector(".rail");
  const handle = $("#railResize");
  if (!handle) return;

  const saved = parseFloat(localStorage.getItem(LS_RAIL_W));
  if (saved && !Number.isNaN(saved)) {
    board.style.setProperty("--rail-w", saved + "px");
  }

  let startX = 0, startW = 0;
  function onMove(e) {
    const dx = startX - e.clientX;
    const w = clamp(startW + dx, 280, window.innerWidth * 0.6);
    board.style.setProperty("--rail-w", w + "px");
    localStorage.setItem(LS_RAIL_W, String(w));
    scheduleMahgenResize();
  }
  function onUp() {
    document.body.style.cursor = "";
    document.body.style.userSelect = "";
    document.removeEventListener("mousemove", onMove);
    document.removeEventListener("mouseup", onUp);
  }
  handle.addEventListener("mousedown", (e) => {
    startX = e.clientX;
    startW = rail.getBoundingClientRect().width;
    document.body.style.cursor = "ew-resize";
    document.body.style.userSelect = "none";
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
    e.preventDefault();
  });
}

function wireBottomBar() {
  const area = $("#bottomArea");
  const handle = $("#bottomResize");
  const toggle = $("#bottomToggle");
  if (!area || !handle || !toggle) return;

  const savedH = parseFloat(localStorage.getItem(LS_BOTTOM_H));
  if (savedH && !Number.isNaN(savedH)) {
    area.style.setProperty("--bottom-h", savedH + "px");
  }
  const userPrefCollapsed = localStorage.getItem(LS_BOTTOM_COLLAPSED);
  if (userPrefCollapsed === "1") area.classList.add("collapsed");

  function paintToggle() {
    toggle.textContent = area.classList.contains("collapsed") ? "▴" : "▾";
  }
  paintToggle();

  let startY = 0, startH = 0;
  function onMove(e) {
    const dy = startY - e.clientY;
    const h = clamp(startH + dy, 80, window.innerHeight * 0.7);
    area.style.setProperty("--bottom-h", h + "px");
    localStorage.setItem(LS_BOTTOM_H, String(h));
  }
  function onUp() {
    document.body.style.cursor = "";
    document.body.style.userSelect = "";
    document.removeEventListener("mousemove", onMove);
    document.removeEventListener("mouseup", onUp);
  }
  handle.addEventListener("mousedown", (e) => {
    if (area.classList.contains("collapsed")) return;
    startY = e.clientY;
    startH = area.getBoundingClientRect().height;
    document.body.style.cursor = "ns-resize";
    document.body.style.userSelect = "none";
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
    e.preventDefault();
  });

  toggle.addEventListener("click", () => {
    const willCollapse = !area.classList.contains("collapsed");
    area.classList.toggle("collapsed", willCollapse);
    localStorage.setItem(LS_BOTTOM_COLLAPSED, willCollapse ? "1" : "0");
    paintToggle();
  });

  // Auto-collapse on tight viewports — but only if the user hasn't pinned a
  // preference. Threshold 600px window height is roughly when there is no
  // room left for both the board and a meaningful bottom panel.
  function applyAuto() {
    const pref = localStorage.getItem(LS_BOTTOM_COLLAPSED);
    if (pref === "1" || pref === "0") return; // user-set, don't override
    if (window.innerHeight < 600) area.classList.add("collapsed");
    else area.classList.remove("collapsed");
    paintToggle();
  }
  applyAuto();
  window.addEventListener("resize", applyAuto);
}

function wireButtons() {
  const safe = (fn) => async () => {
    try { await fn(); }
    catch (e) {
      pushNotif({ level: "error", title: "Error", body: String(e), _time: fmtTime() });
    }
  };

  $("#btnPause").addEventListener("click", safe(async () => {
    pushNotif({ level: "info", title: "Pause toggled (no-op)", _time: fmtTime() });
  }));
  $("#btnReconnect").addEventListener("click", safe(async () => {
    if (HAS_TAURI) {
      try { await invoke("stop_proxy"); } catch {}
      await invoke("start_proxy");
    }
  }));
  $("#btnStopProxy").addEventListener("click", safe(async () => {
    if (HAS_TAURI) await invoke("stop_proxy");
  }));
  $("#qStartProxy").addEventListener("click", safe(async () => {
    if (HAS_TAURI) await invoke("start_proxy");
  }));
  $("#qStopProxy").addEventListener("click", safe(async () => {
    if (HAS_TAURI) await invoke("stop_proxy");
  }));
  $("#qReconnect").addEventListener("click", safe(async () => {
    if (HAS_TAURI) {
      try { await invoke("stop_proxy"); } catch {}
      await invoke("start_proxy");
    }
  }));
  $("#qListBots").addEventListener("click", safe(async () => {
    if (HAS_TAURI) {
      const bots = await invoke("list_bots");
      pushNotif({ level: "info", title: "Bots", body: bots.map((b) => b.name).join(", ") || "(none)", _time: fmtTime() });
    }
  }));
  $("#qSnapshot").addEventListener("click", safe(refreshAll));
  $("#qAnalysis").addEventListener("click", safe(async () => {
    if (HAS_TAURI) {
      const a = await invoke("get_analysis");
      if (a) { state.analysis = a; renderRail(); }
    }
  }));
  $("#btnRefresh").addEventListener("click", safe(refreshAll));
  $("#btnOpenLog").addEventListener("click", safe(async () => {
    if (HAS_TAURI && state.logDir) {
      try { await invoke("open_log_folder"); }
      catch { pushNotif({ level: "info", title: "Log folder", body: state.logDir, _time: fmtTime() }); }
    }
  }));
  $("#btnClearNotif").addEventListener("click", () => {
    state.notifications = [];
    renderNotifications();
  });
}

// =============== Event/state ingest ===============
function pushEvent(e) {
  e._time = fmtTime();
  state.events.push(e);
  if (state.events.length > 100) state.events.shift();
  renderEvents();
}

function pushResponse(r) {
  r._time = fmtTime();
  state.responses.push(r);
  if (state.responses.length > 80) state.responses.shift();
  renderResponses();
}

function pushNotif(n) {
  n._time = n._time ?? fmtTime();
  state.notifications.push(n);
  if (state.notifications.length > 50) state.notifications.shift();
  renderNotifications();
}

async function refreshAll() {
  if (!HAS_TAURI) return;
  try {
    const snap = await invoke("get_status");
    state.config = snap.config;
    state.botStatus = snap.bot_status;
    state.proxyStatus = snap.proxy_status;
    state.logDir = snap.log_dir;
  } catch {}
  try { state.game = await invoke("get_game_snapshot"); } catch {}
  try { state.view = await invoke("get_mahgen_view"); } catch {}
  try { state.analysis = await invoke("get_analysis"); } catch {}
  renderAll();
}

async function subscribeAll() {
  await listenEvent("mjai-event", (ev) => {
    pushEvent(ev.payload);
    // Re-fetch view/game/analysis on a cadence — analysis-result is the canonical sync.
  });
  await listenEvent("bot-response", (ev) => pushResponse(ev.payload));
  await listenEvent("bot-status",  (ev) => { state.botStatus = ev.payload; renderSide(); });
  await listenEvent("proxy-status",(ev) => { state.proxyStatus = ev.payload; renderSide(); });
  await listenEvent("notify",      (ev) => pushNotif(ev.payload));
  await listenEvent("analysis-result", async (ev) => {
    state.analysis = ev.payload;
    try { state.game = await invoke("get_game_snapshot"); } catch {}
    try { state.view = await invoke("get_mahgen_view"); } catch {}
    renderAll();
  });
}

// =============== Mock data (dev preview without Tauri) ===============
function loadMock() {
  state.config = {
    general: { language: "en" },
    logging: { dir: "logs", level: "info", all_level: "info" },
    platform: { kind: "Majsoul" },
    proxy: { enabled: true, addr: "127.0.0.1:11656", ca_dir: ".certs" },
    bot: { enabled: true, active: "mortal", auto_sync: true, dir: "mjai_bot" },
  };
  state.botStatus = { state: "ready", bot: "mortal", actor_id: 0 };
  state.proxyStatus = { state: "running", addr: "127.0.0.1:11656" };
  state.logDir = "/home/user/.akagi/logs/2026-04-27_15-24-11";

  state.game = {
    bakaze: "E",
    kyoku: 1,
    honba: 0,
    kyotaku: 0,
    oya: 1,
    current_player: 2,
    turn_count: 5,
    phase: "wait_act",
    is_done: false,
    our_seat: 2,
    dora_markers: ["2m", "3p"],
    players: [
      { seat: 0, tehai: Array(13).fill("?"), melds: [], river: [], score: 10000, riichi_declared: false, riichi_stage: false, double_riichi: false, riichi_declaration_index: null },
      { seat: 1, tehai: Array(13).fill("?"), melds: [], river: [], score: 10000, riichi_declared: true, riichi_stage: false, double_riichi: false, riichi_declaration_index: 4 },
      { seat: 2, tehai: ["1m","2m","3m","4m","5m","6m","7m","9m","2p","3p","4p","6s","8s"], melds: [], river: [], score: 10000, riichi_declared: false, riichi_stage: false, double_riichi: false, riichi_declaration_index: null },
      { seat: 3, tehai: Array(13).fill("?"), melds: [], river: [], score: 10000, riichi_declared: false, riichi_stage: false, double_riichi: false, riichi_declaration_index: null },
    ],
  };

  state.view = {
    dora_indicators: "2m3p",
    players: [
      { seat: 0, hand: "0000000000000z", melds: ["234m_5p"],     river: "1m^3m^4m^7s_9p2s5z" },
      { seat: 1, hand: "0000000000000z", melds: ["46m_3p"],      river: "4m^6m_4m^9s2z3z" },
      { seat: 2, hand: "1234567m23s9m45p68s", melds: ["123m_5p"], river: "4m^5m^7p^9p^7s^9s5z3m^7m6p1m^" },
      { seat: 3, hand: "0000000000000z", melds: ["234m_4z"],     river: "1m3m5z6p9p1m4s2s7m" },
    ],
  };

  state.analysis = {
    seat: 2,
    turn: 5,
    shanten: 1,
    state: "discard14",
    hand13: null,
    hand14: {
      shanten: 1,
      maintain: [
        { discard: "6m", result: { mixed_waits_score: 1245, mixed_round_point: 0.82, shanten: 1, waits: [], waits_total: 8, next_shanten_waits_count: {}, avg_next_shanten_waits: 0, avg_agari_rate: 0, is_furiten: false, furiten_rate: 0, improves: [], improve_way_count: 0, avg_improve_waits_count: 0, dama_point: 0, riichi_point: 0, yaku_ids: [] } },
        { discard: "9m", result: { mixed_waits_score: 1102, mixed_round_point: 0.67, shanten: 1, waits: [], waits_total: 7, next_shanten_waits_count: {}, avg_next_shanten_waits: 0, avg_agari_rate: 0, is_furiten: false, furiten_rate: 0, improves: [], improve_way_count: 0, avg_improve_waits_count: 0, dama_point: 0, riichi_point: 0, yaku_ids: [] } },
        { discard: "2s", result: { mixed_waits_score: 987,  mixed_round_point: 0.45, shanten: 1, waits: [], waits_total: 6, next_shanten_waits_count: {}, avg_next_shanten_waits: 0, avg_agari_rate: 0, is_furiten: false, furiten_rate: 0, improves: [], improve_way_count: 0, avg_improve_waits_count: 0, dama_point: 0, riichi_point: 0, yaku_ids: [] } },
      ],
      backwards: [],
    },
    opponents: [
      { seat: 0, tenpai_rate: 12.7, risk: synthRisk(18.6), is_riichi: false },
      { seat: 1, tenpai_rate: 24.1, risk: synthRisk(33.2), is_riichi: true  },
      { seat: 3, tenpai_rate: 7.3,  risk: synthRisk(12.8), is_riichi: false },
    ],
    mixed_risk: synthMixedRisk(),
    best_attack_discard: "6m",
    best_defence_discard: "2s",
  };

  state.events = [
    { type: "dahai", actor: 1, pai: "5m", tsumogiri: true,  _time: "15:24:32" },
    { type: "tsumo", actor: 1, pai: "5m",                    _time: "15:24:28" },
    { type: "dahai", actor: 0, pai: "1m", tsumogiri: false, _time: "15:24:24" },
    { type: "pon",   actor: 3, target: 0, pai: "6p", consumed: ["6p","6p"], _time: "15:24:19" },
    { type: "dahai", actor: 3, pai: "4m", tsumogiri: false, _time: "15:24:15" },
  ];

  state.notifications = [
    { level: "success", title: "Proxy started", body: "MITM proxy is running on 127.0.0.1:11656", _time: "15:24:10" },
    { level: "info",    title: "Bot mortal is ready", body: "Actor ID: 0", _time: "15:24:08" },
    { level: "info",    title: "Game started (ID: 0)", body: "You are seat South (2)", _time: "15:24:05" },
  ];

  state.responses = [
    { type: "dahai", pai: "5m", tsumogiri: true,  _time: "15:24:28" },
    { type: "none",                               _time: "15:24:10" },
    { type: "dahai", pai: "2p", tsumogiri: false, _time: "15:24:01" },
    { type: "pon",   pai: "6p", target: 1, consumed: ["6p","6p"], _time: "15:23:52" },
    { type: "chi",   pai: "7s", target: 3, consumed: ["6s","8s"], _time: "15:23:45" },
  ];
}

function synthRisk(peak) {
  const v = new Array(34).fill(0);
  for (let i = 0; i < 34; i++) v[i] = Math.max(0, peak * (0.3 + 0.7 * Math.sin(i * 0.6 + peak)));
  return v;
}
function synthMixedRisk() {
  const v = new Array(34).fill(0);
  for (let i = 0; i < 34; i++) {
    const base = 6 + 18 * Math.abs(Math.sin(i * 0.45));
    v[i] = Math.min(30, base + (i > 22 ? 6 : 0));
  }
  return v;
}

// =============== Init ===============
async function init() {
  initFromStorage();
  applyDom();
  wireNav();
  wireTabs();
  wireLang();
  wireButtons();
  wireRailResize();
  wireBottomBar();

  if (HAS_TAURI) {
    await refreshAll();
    await subscribeAll();
  } else {
    loadMock();
    renderAll();
    // Tag a console hint for the developer
    console.info("[Akagi] not running inside Tauri — mock data loaded.");
  }
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", init);
} else {
  init();
}
