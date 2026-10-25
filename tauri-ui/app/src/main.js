/**
 * PC GamePak — main.js
 *
 * Reads the drive path from the query string (?drive=D%3A%5C), asks the Rust
 * backend what is on the cartridge, and wires up Play and Eject.
 *
 * Backend contract (src-tauri/src/main.rs):
 *   parse_cartridge({ drivePath })            -> { title, cover, cover_path, background, logo, executable, drive_path }
 *   launch_game({ executable, drivePath, title }) -> ()
 *   eject_with_guard({ drivePath })           -> { ejected, message }
 *   cartridge_busy({ drivePath })             -> { holders[], unchecked }
 *   focus_window()                            -> ()
 *   debug_logging() / debug_log(line)         -> diagnostics, off by default
 *   get_settings() / set_settings(settings)   -> Settings
 *   can_eject({ drivePath })                  -> bool
 *   cartridge_health({ drivePath })           -> { link, transport, label, filesystem, usedPercent, warnings[] }
 *   cartridge_stats({ drivePath })            -> { games: { key: { launches, seconds, lastPlayed, … } } }
 *   sync_saves({ drivePath })                 -> SyncOutcome[]  (empty when off)
 *   save_slots({ drivePath })                 -> SlotStatus[]
 *   carried_home({ drivePath })               -> string | null
 *   pull_shaders({ drivePath })               -> Synced[]  (on insert)
 *   shader_slots({ drivePath })               -> ShaderSlot[]
 *
 * `cover` arrives as a data URI already. There is no command that takes a path
 * to read, so the webview cannot ask the backend for arbitrary files.
 *
 * All cartridge-supplied text is written with textContent. The title and paths
 * come off an untrusted volume and must never be able to inject markup.
 */

import { connect as connectGamepad } from "./gamepad.js";
import { createMemoryCard } from "./memcard.js";

const tauri = window.__TAURI__;
const invoke = tauri?.core?.invoke ?? demoInvoke;

const el = {
  card: document.getElementById("card"),
  unbox: document.getElementById("unbox"),
  memcardButton: document.getElementById("btn-memcard"),
  sheetKeys: document.getElementById("sheet-keys"),
  face: document.getElementById("face"),
  slot: document.getElementById("slot"),
  slotLabel: document.getElementById("slot-label"),
  insert: document.getElementById("btn-insert"),
  cartSkin: document.getElementById("cart-skin"),
  cover: document.getElementById("cover-img"),
  coverB: document.getElementById("cover-img-b"),
  placeholder: document.getElementById("cover-placeholder"),
  placeholderInitials: document.getElementById("placeholder-initials"),
  placeholderName: document.getElementById("placeholder-name"),
  eyebrow: document.getElementById("eyebrow"),
  titleLogo: document.getElementById("title-logo"),
  cartMark: document.getElementById("cart-mark"),
  title: document.getElementById("game-title"),
  stage: document.getElementById("stage"),
  notice: document.getElementById("notice"),
  play: document.getElementById("btn-play"),
  playLabel: document.querySelector("#btn-play .btn__label"),
  eject: document.getElementById("btn-eject"),
  close: document.getElementById("btn-close"),
  details: document.getElementById("btn-details"),
  openWizard: document.getElementById("btn-open-wizard"),
  sheet: document.getElementById("sheet"),
  sheetClose: document.getElementById("btn-sheet-close"),
  sheetThumb: document.getElementById("sheet-thumb"),
  sheetTitle: document.getElementById("sheet-title"),
  sheetSub: document.getElementById("sheet-sub"),
  readings: document.getElementById("readings"),
  specs: document.getElementById("specs"),
  paths: document.getElementById("paths"),
  pathsFold: document.getElementById("paths-fold"),
  pathsToggle: document.getElementById("btn-paths"),
  pathsLabel: document.getElementById("paths-label"),
  toast: document.getElementById("toast"),
  gameList: document.getElementById("game-list"),
  playStatsTime: document.getElementById("play-stats-time"),
  playStatsBeat: document.getElementById("play-stats-beat"),
  playStatsLast: document.getElementById("play-stats-last"),
};

let cartridge = null;
/** The connection is only looked at once, and only if the sheet is opened. */
let healthAsked = false;

/* ==========================================================================
   Accent colour, sampled from the cover art
   ========================================================================== */

/** WCAG relative luminance from 8-bit sRGB. */
function luminance(r, g, b) {
  const lin = [r, g, b].map((c) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
}

function contrast(a, b) {
  const [hi, lo] = a > b ? [a, b] : [b, a];
  return (hi + 0.05) / (lo + 0.05);
}

function hslToRgb(h, s, l) {
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const hp = (((h % 360) + 360) % 360) / 60;
  const x = c * (1 - Math.abs((hp % 2) - 1));
  const [r, g, b] =
    hp < 1 ? [c, x, 0]
    : hp < 2 ? [x, c, 0]
    : hp < 3 ? [0, c, x]
    : hp < 4 ? [0, x, c]
    : hp < 5 ? [x, 0, c]
    : [c, 0, x];
  const m = l - c / 2;
  return [(r + m) * 255, (g + m) * 255, (b + m) * 255];
}

/** The label contrast of an accent at lightness `l`, with the better ink. */
function inkFor(h, s, l) {
  const accentLum = luminance(...hslToRgb(h, s, l));
  const dark = contrast(accentLum, luminance(...hslToRgb(h, 0.22, 0.11)));
  const light = contrast(accentLum, luminance(...hslToRgb(h, 0.12, 0.97)));
  return dark >= light
    ? { ratio: dark, ink: `hsl(${h.toFixed(0)} 22% 11%)` }
    : { ratio: light, ink: `hsl(${h.toFixed(0)} 12% 97%)` };
}

/**
 * Apply an accent and pick the ink that sits on it.
 *
 * Play is the only filled surface in the window, so its label has to stay
 * readable whatever colour the artwork happens to be: dark ink wins on ambers
 * and greens, near-white on deep blues and reds.
 *
 * Choosing the better of the two inks is not enough by itself. At the sampler's
 * preferred lightness the best ink still measures about 4.0:1 on deep blues,
 * purples and crimsons — under AA for a 12px uppercase label, and those are
 * exactly the colours game covers are made of. Rather than give up the sampled
 * colour and fall back to a stock amber, the hue and saturation stay as the
 * artwork gave them and only the lightness moves, the smallest step first,
 * until the measurement clears. The target carries a little headroom because
 * the button paints a gradient over this colour.
 */
function setAccent(h, s, preferred) {
  const TARGET = 4.6;
  const root = document.documentElement.style;

  let best = { lightness: preferred, ...inkFor(h, s, preferred) };

  for (let step = 0.02; best.ratio < TARGET && step <= 0.34; step += 0.02) {
    for (const l of [preferred - step, preferred + step]) {
      if (l < 0.3 || l > 0.9) continue;
      const candidate = inkFor(h, s, l);
      if (candidate.ratio > best.ratio) best = { lightness: l, ...candidate };
      if (best.ratio >= TARGET) break;
    }
  }

  root.setProperty(
    "--accent",
    `hsl(${h.toFixed(0)} ${(s * 100).toFixed(0)}% ${(best.lightness * 100).toFixed(1)}%)`,
  );
  root.setProperty("--accent-ink", best.ink);
}

/**
 * Pick the colour a person would name if asked to describe the cover.
 *
 * A flat average of any cover is mud, so each pixel is weighted by its own
 * saturation squared, biased toward the lit areas. Hues are summed as vectors
 * so reds either side of 0° do not cancel out.
 */
function sampleAccent(img) {
  const SIZE = 48;
  const canvas = document.createElement("canvas");
  canvas.width = SIZE;
  canvas.height = SIZE;
  // Read once, so the CPU-backed canvas that willReadFrequently asks for would
  // cost the draw and buy nothing back.
  const ctx = canvas.getContext("2d");
  if (!ctx) return;

  ctx.drawImage(img, 0, 0, SIZE, SIZE);
  let data;
  try {
    data = ctx.getImageData(0, 0, SIZE, SIZE).data;
  } catch {
    return; // tainted canvas: keep the default accent
  }

  let x = 0;
  let y = 0;
  let satSum = 0;
  let weight = 0;

  for (let i = 0; i < data.length; i += 4) {
    if (data[i + 3] < 200) continue;
    const r = data[i] / 255;
    const g = data[i + 1] / 255;
    const b = data[i + 2] / 255;
    const max = Math.max(r, g, b);
    const min = Math.min(r, g, b);
    const l = (max + min) / 2;
    const d = max - min;
    if (d < 0.06) continue; // greys carry no hue
    if (l < 0.08 || l > 0.95) continue; // crushed and blown pixels lie about hue

    const s = d / (1 - Math.abs(2 * l - 1));
    let h = 0;
    if (max === r) h = 60 * (((g - b) / d) % 6);
    else if (max === g) h = 60 * ((b - r) / d + 2);
    else h = 60 * ((r - g) / d + 4);

    const w = s * s * (0.3 + l);
    const rad = (h * Math.PI) / 180;
    x += Math.cos(rad) * w;
    y += Math.sin(rad) * w;
    satSum += s * w;
    weight += w;
  }

  // A greyscale cover gets the stock accent back. Returning without touching
  // it kept whatever the previous game on a collection had set, so a black and
  // white game's Play stayed the colour of the one above it on the rail.
  if (weight === 0) {
    document.documentElement.style.removeProperty("--accent");
    document.documentElement.style.removeProperty("--accent-ink");
    return;
  }

  const hue = (Math.atan2(y, x) * 180) / Math.PI;
  // Floor the saturation and hold the lightness out of the pastel range so the
  // sampled accent always has enough body to carry a button.
  const saturation = Math.min(Math.max(satSum / weight, 0.72), 0.9);
  setAccent(hue, saturation, 0.575);
}

/* ==========================================================================
   The slot, and the cartridge seated in it
   ========================================================================== */

/** How long the face takes to leave or enter the slot; matches style.css. */
const SEAT_MS = 620;

/**
 * Show what is behind the cartridge.
 *
 * The empty slot is not decoration: it is what the window has to say when
 * there is no readable cartridge to show, and what Eject leaves behind. The
 * label carries the reason, so nothing has to be explained twice.
 */
function showSlot(label, action) {
  el.slotLabel.textContent = label;
  if (action) {
    el.insert.textContent = action.label;
    el.insert.hidden = false;
    el.insert.onclick = action.onClick;
  } else {
    el.insert.hidden = true;
    el.insert.onclick = null;
  }
  el.card.classList.add("is-ejected");
  // Nothing on the face is reachable once it has left the slot.
  el.face.inert = true;
}

/* ==========================================================================
   Unboxing
   ========================================================================== */

/** Just past the last animation in style.css, in case its end is never seen. */
const UNBOX_MS = 2750;

/** Set while the unboxing plays: calling it ends the unboxing early. */
let unboxing = null;

/**
 * Open the box, the first time this machine sees this cartridge.
 *
 * Asked of the backend, which records the answer, so it plays once per
 * cartridge per PC. Skipped outright under reduced motion — the stylesheet
 * switches animations off there, and a box that never opens is worse than none
 * — and for a skin that says `--skin-unbox: none`.
 */
async function maybeUnbox(art) {
  const style = getComputedStyle(document.documentElement);
  if (style.getPropertyValue("--skin-unbox").trim() === "none") return;
  let first = false;
  try {
    first = tauri
      ? await invoke("first_insert", { title: cartridge?.title ?? "" })
      : new URLSearchParams(location.search).has("unbox");
  } catch {
    return;
  }
  if (!first || matchMedia("(prefers-reduced-motion: reduce)").matches) return;

  const game = currentGame();
  const front = isCollection() ? cartridge?.cover || art : art;
  const logo = isCollection() ? cartridge?.logo || game?.logo : cartridge?.logo;
  showImage(el.unbox.querySelector(".unbox__front"), front);
  showImage(el.unbox.querySelector(".unbox__label-art"), art);
  showImage(el.unbox.querySelector(".unbox__logo"), logo);
  el.unbox.querySelector(".unbox__title").textContent = cartridge?.title || game?.title || "";

  el.unbox.hidden = false;
  el.unbox.classList.add("is-playing");
  await new Promise((resolve) => {
    const timer = setTimeout(() => unboxing?.(), UNBOX_MS);
    unboxing = () => {
      clearTimeout(timer);
      unboxing = null;
      resolve();
    };
  });
  el.unbox.classList.remove("is-playing");
  el.unbox.hidden = true;
}

function showImage(img, src) {
  const usable = Boolean(src) && /^(data:image\/|src\/)/.test(String(src));
  img.hidden = !usable;
  if (usable) img.src = src;
  else img.removeAttribute("src");
}

// Any key, click or pad button skips it, and does nothing else: the Enter that
// skips the box must not also press Play on the cartridge that comes out.
window.addEventListener(
  "keydown",
  (event) => {
    if (!unboxing) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    unboxing();
  },
  { capture: true },
);
el.unbox.addEventListener("pointerdown", () => unboxing?.());

/** Seat the cartridge: the entrance, mirroring the insert that just happened. */
function seat() {
  el.face.inert = false;
  el.card.classList.remove("is-ejected");
}

/* ==========================================================================
   Details sheet
   ========================================================================== */

function specRow(list, label, value, muted = false) {
  const dt = document.createElement("dt");
  dt.textContent = label;
  const dd = document.createElement("dd");
  dd.textContent = value;
  // Mount points and launch targets are what people paste into a terminal or a
  // bug report, so they opt out of the window-wide selection ban.
  dd.classList.add("selectable");
  if (muted) dd.classList.add("is-muted");
  const row = document.createElement("div");
  row.append(dt, dd);
  list.append(row);
}

/**
 * A reading, with its advisory folded behind its own icon.
 *
 * The two things the sheet is opened for are whether the link is healthy and
 * whether the drive is nearly full. Everything that explains a reading belongs
 * to that reading, not to a block of prose under the whole sheet, so it hides
 * behind the icon on the card it is about and the sheet stays quiet until it
 * is asked why.
 */
function reading({ label, value, unit, note, fill, advisory, icon }) {
  const card = document.createElement("div");
  card.className = "reading";

  const head = document.createElement("div");
  head.className = "reading__head";
  const labelEl = document.createElement("p");
  labelEl.className = "reading__label";
  labelEl.textContent = label;
  head.append(labelEl);

  let box;
  if (advisory) {
    const why = document.createElement("button");
    why.className = "reading__why";
    why.type = "button";
    why.setAttribute("aria-expanded", "false");
    why.setAttribute("aria-label", `Why ${label.toLowerCase()} matters`);
    why.title = "Why this matters";
    why.innerHTML = icon === "warn"
      ? '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" width="11" height="11" aria-hidden="true"><path d="M8 2.6L14.6 13.4H1.4z" /><path d="M8 6.6v3.1M8 11.6v.1" /></svg>'
      : '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" width="11" height="11" aria-hidden="true"><circle cx="8" cy="8" r="6.4" /><path d="M8 5.1v.1M8 7.4v3.6" /></svg>';
    head.append(why);

    box = document.createElement("div");
    box.className = "reading__advisory";
    const p = document.createElement("p");
    // The lead is the reading restated; the rest is why it matters.
    const lead = document.createElement("strong");
    lead.textContent = advisory.lead;
    p.append(lead, ` ${advisory.body}`);
    box.append(p);

    why.addEventListener("click", () => {
      const open = box.classList.toggle("is-open");
      why.setAttribute("aria-expanded", String(open));
    });
  }

  card.append(head);

  const valueEl = document.createElement("p");
  valueEl.className = "reading__value";
  valueEl.textContent = value;
  if (unit) {
    const unitEl = document.createElement("span");
    unitEl.className = "reading__unit";
    unitEl.textContent = ` ${unit}`;
    valueEl.append(unitEl);
  }
  card.append(valueEl);

  if (note) {
    const noteEl = document.createElement("p");
    noteEl.className = "reading__note";
    noteEl.textContent = note;
    card.append(noteEl);
  }

  if (typeof fill === "number") {
    const bar = document.createElement("div");
    bar.className = "reading__bar";
    const inner = document.createElement("div");
    inner.className = "reading__fill";
    inner.style.width = `${Math.max(0, Math.min(100, fill))}%`;
    bar.append(inner);
    card.append(bar);
  }

  if (box) card.append(box);
  return card;
}

/** Split "9.7 GB" into the number and its unit, so the number can be set large. */
function splitMeasure(text) {
  const match = /^(\S+)\s*(.*)$/.exec(text ?? "");
  return match ? { value: match[1], unit: match[2] } : { value: text ?? "—", unit: "" };
}

function renderIdentity(info) {
  el.sheetTitle.textContent = info.title || "Cartridge";
  if (info.cover) {
    el.sheetThumb.src = info.cover;
    el.sheetThumb.hidden = false;
  } else {
    el.sheetThumb.hidden = true;
    el.sheetThumb.removeAttribute("src");
  }
}

/**
 * The paths. Least-read thing on the sheet, so they start folded.
 *
 * A collection carries one launch target per game rather than one for the
 * cartridge, so the list says exactly what each row of the rail starts.
 */
function renderPaths(info) {
  el.paths.replaceChildren();
  const games = info.games ?? [];

  if (games.length > 1) {
    for (const game of games) {
      specRow(el.paths, game.title || "Untitled", game.executable || "nothing configured", !game.executable);
    }
  } else {
    specRow(el.paths, "Launch", info.executable || "nothing configured", !info.executable);
  }

  specRow(el.paths, "Drive", info.drive_path || "—", !info.drive_path);
  specRow(el.paths, "Cover", info.cover_path || "none found", !info.cover_path);
}

function togglePaths(open) {
  // The fold opens; the list inside it only holds the rows.
  const next = open ?? !el.pathsFold.classList.contains("is-open");
  el.pathsFold.classList.toggle("is-open", next);
  el.pathsFold.hidden = !next;
  el.pathsToggle.setAttribute("aria-expanded", String(next));
  el.pathsLabel.textContent = next ? "Hide file paths" : "Show file paths";
}

/**
 * How the cartridge is connected, filled in after the sheet is open.
 *
 * Three things decide whether a cartridge performs like the drive inside it —
 * the negotiated link, UASP or BOT, and how full it is — and none of them are
 * visible anywhere else. Asked for lazily: on Windows this shells out, and the
 * launcher opening late is worse than the sheet filling in late.
 */
async function renderHealth() {
  if (!cartridge?.drive_path || healthAsked) return;
  healthAsked = true;

  let health;
  try {
    health = await invoke("cartridge_health", { drivePath: cartridge.drive_path });
  } catch {
    return; // the sheet is still useful without it
  }

  // The drive's own name, so the sheet says which cartridge this is without
  // anyone reading a mount path.
  const marks = [health.label, health.filesystem].filter(Boolean);
  el.sheetSub.textContent = marks.join(" · ");

  const cards = [];

  if (health.link) {
    const { value, unit } = splitMeasure(health.link);
    const slow = typeof health.linkMbps === "number" && health.linkMbps < 10_000;
    cards.push(
      reading({
        label: "Link",
        value,
        unit,
        // The protocol's name means nothing to a player; what it costs does.
        note:
          { UASP: "Fast mode (UASP)", BOT: "Slow mode (BOT)" }[health.transport] ||
          health.transport ||
          undefined,
        icon: slow ? "warn" : "info",
        advisory: advisoryForLink(health),
      }),
    );
  }

  if (health.totalBytes > 0) {
    const { value, unit } = splitMeasure(formatBytes(health.freeBytes));
    cards.push(
      reading({
        label: "Free",
        value,
        unit,
        fill: health.usedPercent,
        icon: "warn",
        advisory: advisoryForSpace(health),
      }),
    );
  }

  el.readings.replaceChildren(...cards);
}

/**
 * The link advisory, rewritten for a card rather than a paragraph.
 *
 * The backend's `warnings` are full sentences meant to stand alone. Here the
 * reading is already on screen directly above, so the lead restates it in three
 * words and the body is only the part the number cannot say.
 */
function advisoryForLink(health) {
  if (typeof health.linkMbps === "number" && health.linkMbps < 1000) {
    return {
      lead: "USB 2.0 port or cable.",
      body: "Games will stream badly. Try a different port, and the cable the enclosure came with.",
    };
  }
  if (typeof health.linkMbps === "number" && health.linkMbps < 10_000) {
    return {
      lead: "About half what the enclosure can do.",
      body: "Usually a front-panel port, a hub, or a cable that is not rated for 10 Gbps.",
    };
  }
  if (health.transport === "BOT") {
    return {
      lead: "One command at a time.",
      body: "UASP would queue them — worth 2–3× on the small random reads a game streams. A different port or enclosure usually fixes it.",
    };
  }
  return null;
}

function advisoryForSpace(health) {
  if (health.usedPercent < 85) return null;
  return {
    lead: `${health.usedPercent}% full.`,
    body: "No DRAM of its own and no host memory over USB, so the last 15% costs more than it looks like. Leaving room back keeps random reads quick.",
  };
}

function formatBytes(bytes) {
  if (!Number.isFinite(bytes) || bytes <= 0) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let value = bytes;
  while (value >= 1000 && i < units.length - 1) {
    value /= 1000;
    i += 1;
  }
  return `${value.toFixed(value >= 100 || i === 0 ? 0 : 1)} ${units[i]}`;
}

function toggleSheet(open) {
  const next = open ?? !el.sheet.classList.contains("is-open");
  if (next) renderHealth();
  el.sheet.classList.toggle("is-open", next);
  el.sheet.hidden = !next;
  el.details.setAttribute("aria-expanded", String(next));

  // The sheet is opaque and covers the card, but Play and Eject stay in the tab
  // order behind it unless they are made inert — and a Play button you cannot
  // see is a game launched from what looks like a details screen. The toast is
  // left reachable so a message that arrives underneath can still be read.
  for (const node of el.card.children) {
    if (node !== el.sheet && node !== el.toast) node.inert = next;
  }

  // preventScroll matters here: #card has overflow:hidden, which makes it a
  // scroll container, and focusing the sheet while it is still translated
  // off-screen would scroll the whole card up to reveal it.
  if (next) el.sheetClose.focus({ preventScroll: true });
  else el.details.focus({ preventScroll: true });
  el.card.scrollTop = 0;
}

/* ==========================================================================
   Status
   ========================================================================== */

let toastTimer;

/**
 * Say something, briefly.
 *
 * Confirmations leave on their own. An error does not: a failed launch is the
 * one message in this window a person needs to read twice, and 3.4 seconds is
 * gone before a slow reader reaches the end of it. It waits for a click, for
 * Escape, or for whatever happens next.
 */
function toast(message, isError = false) {
  el.toast.textContent = message;
  el.toast.classList.toggle("is-error", isError);
  el.toast.classList.toggle("is-sticky", isError);
  el.toast.classList.add("is-on");
  clearTimeout(toastTimer);
  if (!isError) {
    toastTimer = setTimeout(() => el.toast.classList.remove("is-on"), 3400);
  }
}

function dismissToast() {
  clearTimeout(toastTimer);
  el.toast.classList.remove("is-on", "is-sticky");
}

el.toast.addEventListener("click", dismissToast);

/**
 * Whether this cartridge is on a drive at all.
 *
 * A cartridge can also be a plain directory that never moves — it resolves to
 * machine, so there is nothing to unmount and no button to offer. The backend
 * refuses the command as well — this only keeps the window honest.
 */
let ejectable = true;

function setBusy(busy) {
  el.play.disabled = busy || !playable();
  el.eject.disabled = busy || !cartridge || !ejectable;
  if (el.gameList && !el.gameList.hidden) {
    for (const row of el.gameList.querySelectorAll(".game-row")) {
      row.setAttribute("aria-disabled", String(busy));
    }
  }
}

async function closeWindow() {
  if (tauri?.window) await tauri.window.getCurrentWindow().close();
}

/**
 * Get out of the way of the game without ending the session.
 *
 * Closing here used to be right: the launcher had said everything it had to
 * say. But Eject is on this window, and quitting a game is exactly when a
 * person wants it — so closing meant unplugging the cartridge and plugging it
 * back in to reach the button that exists to make that unnecessary.
 *
 * Always-on-top goes with it. It is there so the popup lands in front of
 * whatever is running; keeping it while a game is running would put the
 * launcher over the game the moment anything restored it.
 */
async function standAside() {
  if (!tauri?.window) return;
  const self = tauri.window.getCurrentWindow();
  try {
    await self.setAlwaysOnTop(false);
  } catch {
    // Older webview, or a platform without it. Minimising still works.
  }
  await self.minimize();
  resetPlay();
}

/* ==========================================================================
   The rail: which game Play acts on
   ========================================================================== */

/** Index into cartridge.games; 0 for a single game. */
let selected = 0;

/** Which layer is currently showing, so the other one can take the next art. */
let coverFront = true;

function games() {
  return cartridge?.games ?? [];
}

function isCollection() {
  return Boolean(cartridge?.is_bundle) && games().length > 0;
}

function currentGame() {
  return isCollection() ? games()[selected] : cartridge;
}

function playable() {
  return Boolean(currentGame()?.executable);
}

/**
 * Move the selection.
 *
 * A collection has one Play, and it acts on whatever the rail has picked — so
 * picking a game is what changes the artwork, the title and what Play will
 * start. Giving every row its own button made the window a list of buttons;
 * this makes it a cartridge with a side selected.
 */
function select(index) {
  const list = games();
  if (index < 0 || index >= list.length || index === selected) return;
  selected = index;

  for (const row of el.gameList.querySelectorAll(".game-row")) {
    row.setAttribute("aria-selected", String(Number(row.dataset.index) === index));
  }

  const row = el.gameList.querySelector(`.game-row[data-index="${index}"]`);

  // The sheet's Played row is about the selected game, so it moves with the
  // rail rather than staying on whatever was picked when the sheet was drawn.
  if (cartridge) renderSpecs(cartridge);
  exposePlay(el.card, list[index]);

  // Focus follows the selection whenever it is already in the list. A row
  // clicked with the mouse keeps the focus ring, so arrowing away from it left
  // two rows lit in two different styles — the ring on the clicked one and the
  // selected state on the real one — with only the second of them true.
  if (document.activeElement?.classList.contains("game-row")) row?.focus();
  row?.scrollIntoView({ block: "nearest" });

  const game = list[index];
  renderLogo();
  const art = artFor(game);
  if (art) crossfadeCover(art);
  else showPlaceholder(game);
  setBusy(false);
}


/** Which way each arrow key points, in the same [x, y] the pad reports. */
const ARROWS = {
  ArrowUp: [0, -1],
  ArrowDown: [0, 1],
  ArrowLeft: [-1, 0],
  ArrowRight: [1, 0],
};

/**
 * How many games are on a row, right now.
 *
 * Measured rather than declared, because it is the skin's business: a list is
 * one, a grid is however many fit across, and a horizontal strip is all of
 * them. Rows sharing a top edge are on the same row, which holds for a flex
 * column, a flex row and a grid without any of them having to say so.
 */
function columns() {
  const rows = [...el.gameList.querySelectorAll(".game-row")];
  if (rows.length < 2) return 1;
  const first = rows[0].offsetTop;
  return Math.max(1, rows.filter((row) => row.offsetTop === first).length);
}

/**
 * Move the selection, wrapping.
 *
 * Both axes go through here: the pad reports a direction and this turns it into
 * a distance using the row width above, so down means the next row in a grid
 * and the next game in a list, with no skin having to be asked which it is.
 *
 * Wraps because a shelf of games on a television is a ring, not a form:
 * pressing up on the first game should reach the last rather than stop.
 */
function step(delta) {
  const list = games();
  if (list.length < 2) return;
  select((selected + delta + list.length) % list.length);
}

/**
 * One press of a direction, from the keys or the pad.
 *
 * A rail that is a single row — a horizontal strip — has no row above or
 * below, so a vertical step of a whole row landed back on the same game and up
 * and down did nothing. There they move along the strip instead.
 */
function move(x, y) {
  const across = columns();
  step(x + y * (across >= games().length ? 1 : across));
}

/** Which of a game's four pictures each name asks for. `grid` is the cover. */
const ART_FIELD = {
  hero: "background",
  grid: "cover",
  logo: "logo",
  icon: "icon",
};

/**
 * The kind of picture one of the skin's art properties asked for.
 *
 * There are three slots rather than one, because a window showing a picture in
 * three places wants three different pictures in them — a hero behind
 * everything, covers in the rail, and the selected game's logo printed in
 * front. That is what the stock launcher already does for a single game, and
 * with one property a skin could only ask for the same picture three times.
 *
 *   :root {
 *     --skin-art: hero;        the fill behind everything
 *     --skin-row-art: grid;    each row's thumbnail
 *     --skin-logo: game;       the logo printed over the fill
 *   }
 *
 * A slot left unset falls back to `--skin-art`, so a skin that wants one kind
 * throughout still says it once and a skin written before the split behaves
 * exactly as it did. Anything unrecognised gets the grid, and `none` means no
 * picture at all — worth saying for a rail of names with no thumbnails, which
 * would otherwise build an image element per row to show nothing.
 */
function artKind(property) {
  const style = getComputedStyle(document.documentElement);
  const own = style.getPropertyValue(property).trim();
  if (own === "none" || own in ART_FIELD) return own;

  const fill = style.getPropertyValue("--skin-art").trim();
  return fill === "none" || fill in ART_FIELD ? fill : "grid";
}

/**
 * The picture that fills the window, for whichever game is showing.
 *
 * All four are on offer, the icon included: it exists for autorun.inf and makes
 * a poor fill at 256px, but it is a skin maker's to misuse if they have a
 * reason.
 *
 * The game's own picture is preferred over the cartridge's, then the game's
 * cover, then the cartridge's — so somebody filling in heroes for ten games and
 * leaving the rest empty gets exactly what they asked for, and somebody who
 * filled in none still gets a cartridge that looks like something.
 */
function artFor(game) {
  const kind = artKind("--skin-art");
  if (kind === "none") return "";

  const field = ART_FIELD[kind];
  const pick = (from) => (from ? from[field] : "") || "";

  return pick(game) || pick(cartridge) || game?.cover || cartridge?.cover || "";
}

/**
 * Say which game it is when there is no picture for it.
 *
 * Initials rather than a generic icon. Ten games under a skin that wants heroes
 * with three heroes filled in is seven blank frames, and seven frames that each
 * say which game they are is a cartridge somebody can still use.
 */
function showPlaceholder(game) {
  const title = game?.title || cartridge?.title || "";
  el.placeholderInitials.textContent = initialsOf(title);
  el.placeholderName.textContent = title;
  el.cover.hidden = true;
  el.coverB.hidden = true;
  el.placeholder.classList.remove("hidden");
}

/**
 * The picture for one row, which is that game's or nothing.
 *
 * Deliberately not `artFor`: that one falls back to the cartridge's art so the
 * window is never blank, and a list of ten rows doing the same would be ten
 * copies of one cover. A row with no picture says which game it is instead.
 */
function rowArtFor(game) {
  const kind = artKind("--skin-row-art");
  if (kind === "none") return "";
  return game[ART_FIELD[kind]] || game.cover || "";
}

/**
 * How big this game is, in four words a stylesheet can match on.
 *
 * The size is already printed on the row as text, which a skin can style but
 * cannot lay out from — CSS has no way to read "64.2 GB" and turn it into a
 * column span. A band on the element is the smallest thing that lets a skin
 * treat a 60 GB install differently from a 2 GB one: a wider box on a shelf, a
 * taller module in a rack.
 *
 *   .game-row[data-size="large"] { grid-column: span 2 }
 *
 * Bands rather than the number itself, because a skin wants three or four
 * cases and not a scale, and because the boundaries can move here without
 * every skin that used them having to change.
 */
function sizeBand(bytes) {
  if (!bytes) return "none";
  const gb = bytes / 1e9;
  if (gb >= 40) return "large";
  if (gb >= 12) return "medium";
  return "small";
}

/**
 * A short mark for a game with no picture.
 *
 * Read from the name before any subtitle, so "FTL: Faster Than Light" is FTL
 * and not FFT, the initials of "FTL:", "Faster" and "Than". A name that is
 * already an abbreviation is kept whole; otherwise it is the first letter of up
 * to three words. The letter is the first letter or digit in the word, so
 * "(2000)" gives 2 rather than a bracket.
 */
function initialsOf(title) {
  const full = (title || "").trim();
  const main = full.split(/\s*[:–—]\s*|\s+-\s+/)[0] || full;
  const words = main.split(/\s+/).filter((word) => /[\p{L}\p{N}]/u.test(word));
  if (words.length === 1 && /^[\p{Lu}\p{N}]{2,4}$/u.test(words[0])) {
    return words[0];
  }
  return words
    .slice(0, 3)
    .map((word) => word.match(/[\p{L}\p{N}]/u)[0].toUpperCase())
    .join("");
}

/** Bring the next game's art up behind the current one, then swap. */
function crossfadeCover(src) {
  const incoming = coverFront ? el.coverB : el.cover;
  incoming.hidden = false;
  incoming.src = src;
  coverFront = !coverFront;
  el.card.classList.toggle("is-crossfaded", !coverFront);

  // The accent belongs to the art that is showing. It was sampled once, at
  // startup, from whichever game loaded first — so on a collection Play kept
  // that game's colour however far down the rail you went, and if the first
  // cover was too grey to sample it kept the stock amber for ever.
  const resample = () => sampleAccent(incoming);
  if (incoming.complete && incoming.naturalWidth > 0) resample();
  else incoming.addEventListener("load", resample, { once: true });
}

function renderRail(list) {
  el.gameList.replaceChildren();

  list.forEach((game, index) => {
    const row = document.createElement("li");
    row.className = "game-row";
    row.setAttribute("role", "option");
    row.setAttribute("aria-selected", String(index === selected));
    row.tabIndex = 0;
    row.dataset.index = String(index);
    row.dataset.size = sizeBand(game.sizeBytes);
    exposePlay(row, game);

    // A row is that game, so it shows that game's picture of whichever kind the
    // skin asked for and stops there — falling through to the cartridge's would
    // fill a grid of ten shortcuts with ten copies of the same cover.
    const art = rowArtFor(game);
    let img;
    if (art) {
      img = document.createElement("img");
      img.className = "game-row__cover";
      img.alt = "";
      img.src = art;
    } else {
      // Its initials instead, so a game with no picture is still a game rather
      // than an empty square.
      img = document.createElement("span");
      img.className = "game-row__cover game-row__cover--empty";
      img.textContent = initialsOf(game.title);
      img.setAttribute("aria-hidden", "true");
    }

    const body = document.createElement("span");
    body.className = "game-row__body";
    const titleEl = document.createElement("span");
    titleEl.className = "game-row__title";
    titleEl.textContent = game.title || "Unknown game";
    const meta = document.createElement("span");
    meta.className = "game-row__meta";
    // Only a game whose files are actually on the cartridge has a size worth
    // printing; a key that points at an installed copy takes no room.
    // Left empty in the stock look: the size does not help choose what to
    // play. The element stays, and the row keeps data-size, for skins.
    meta.textContent = "";
    body.append(titleEl, meta);

    const dot = document.createElement("span");
    dot.className = "game-row__dot";
    dot.setAttribute("aria-hidden", "true");

    row.append(img, body, dot);
    row.addEventListener("click", () => select(index));
    row.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        select(index);
      }
    });
    el.gameList.append(row);
  });
}

/* ==========================================================================
   Boot
   ========================================================================== */

/**
 * Which cartridge this window is for.
 *
 * Inside Tauri the answer comes from the backend, which has the `--drive`
 * argument it was started with. The query string is only for the browser
 * preview, where there is no backend to ask.
 */
async function resolveDrivePath() {
  if (tauri) {
    const fromArgs = await invoke("drive_path");
    if (fromArgs) return fromArgs;
  }
  return new URLSearchParams(location.search).get("drive") ?? "";
}

/**
 * There is nothing to show.
 *
 * The window stays a slot rather than a cartridge with an error printed on it:
 * an unreadable cartridge is an empty slot, and saying so in the slot's own
 * label needs no second explanation underneath.
 */
function fail(headline, detail) {
  el.card.classList.add("is-blocked");
  showSlot(headline, { label: "Close", onClick: closeWindow });
  toast(`${detail} Close this window, reconnect the cartridge, and try again.`, true);
}

/** The name printed under the mark. A collection repoints it on every pick. */
function setGameTitle(title) {
  const safeTitle = title || "Unknown game";
  el.title.textContent = safeTitle;
  el.title.classList.toggle("is-long", safeTitle.length > 15);
}

/**
 * Print the cartridge's mark and the name under it.
 *
 * `logoOf` is what separates a single game from a collection. A single game's
 * logo *is* its title, so it stands in for the heading rather than repeating
 * the name underneath itself — that is what `null` means. A collection's mark
 * names the collection, so the heading below it has to stay: it is the only
 * thing that says which game the rail has picked.
 */
/**
 * Point the printed logo at whatever the skin asked for.
 *
 *   --skin-logo: auto    the game's own logo instead of the heading — on a
 *                        collection, the selected game's, so the logo moves
 *                        with the rail; a game without one keeps the heading
 *                        (the stock behaviour, and the default)
 *   --skin-logo: game    the same, except that a game with no logo of its own
 *                        borrows the collection's
 *   --skin-logo: none    no logo; the heading is type
 *
 * Called again on every pick, so the logo follows the rail rather than being
 * decided once when the cartridge loaded.
 */
function renderLogo() {
  const game = currentGame();
  const collected = isCollection();
  const mode = getComputedStyle(document.documentElement)
    .getPropertyValue("--skin-logo")
    .trim();

  // A collection prints the selected game's own logo, so each game names
  // itself as the rail moves; one without a logo falls back to the heading.
  // The collection's logo is never printed over a game — that put Mirror's
  // Edge's logo across Hollow Knight — unless a skin asks for `game` and the
  // game has none.
  const logo =
    mode === "none"
      ? null
      : collected
        ? game?.logo || (mode === "game" ? cartridge?.logo : null) || null
        : mode === "game"
          ? game?.logo || cartridge?.logo || null
          : cartridge?.logo || null;

  renderTitle(game?.title ?? cartridge?.title, logo);
}

function renderTitle(title, logo, logoOf = null) {
  setGameTitle(title);

  if (!el.titleLogo) return;
  if (logo && String(logo).startsWith("data:image/")) {
    el.titleLogo.src = logo;
    el.titleLogo.alt = `${logoOf ?? title ?? "Unknown game"} logo`;
    el.titleLogo.hidden = false;
    el.stage?.classList.toggle("has-logo", logoOf === null);
    return;
  }

  el.titleLogo.hidden = true;
  el.titleLogo.removeAttribute("src");
  el.titleLogo.alt = "";
  el.stage?.classList.remove("has-logo");
}

async function init() {
  // The window opens on an empty slot and the cartridge seats into it, which is
  // the insert that has just happened in the user's hand.
  el.card.classList.add("is-ejected");
  el.face.inert = true;

  const nfcSelection = await invoke("nfc_selection").catch(() => false);
  let drivePath;
  try {
    drivePath = await resolveDrivePath();
  } catch (error) {
    fail("GamePak unavailable", String(error));
    await showWindow();
    return;
  }

  if (!drivePath) {
    fail("No cartridge", "The launcher was started without a --drive path.");
    await showWindow();
    return;
  }
  memcardDrive = drivePath;

  // A memory card carries saves and no game, so the card is the whole window.
  let card = null;
  try {
    card = await invoke("memory_card", { drivePath });
  } catch (error) {
    debugLog(`memory card: ${error}`);
  }
  if (card?.cardOnly) {
    el.card.classList.add("is-memcard-only");
    await memcard.show(drivePath, closeWindow);
    await showWindow();
    return;
  }

  try {
    cartridge = await invoke("parse_cartridge", { drivePath });
  } catch (error) {
    fail("Unreadable", String(error));
    await showWindow();
    return;
  }

  // Before the window is shown, so the cartridge never appears in the stock
  // look and then changes its mind a frame later.
  wearSkin(cartridge.skin_css ?? "");
  // What the cartridge is for (`platform=`), for a skin: `#card[data-platform="snes"]`.
  el.card.dataset.platform = String(cartridge.platform || "PC").toLowerCase();

  // A directory has no drive behind it, so Eject goes away rather than failing when
  // pressed. If the backend cannot answer, assume there is a drive: an old
  // build that does not know the command should keep the button it had.
  try {
    ejectable = Boolean(await invoke("can_eject", { drivePath }));
  } catch {
    ejectable = true;
  }
  if (!ejectable) el.eject.hidden = true;

  // A collection's mark names the collection, so the heading under it stays
  // free to name whichever game the rail has picked.
  const collected = isCollection();
  renderLogo();
  if (collected && cartridge.logo) {
    el.cartMark.src = cartridge.logo;
    el.cartMark.alt = `${cartridge.title || "Collection"} logo`;
    el.cartMark.hidden = false;
  }
  renderIdentity(cartridge);
  // A combo drive: the same saves, as a memory card, one button away.
  el.memcardButton.hidden = !cartridge.memory_card;
  // Opened by a front-end's "Memory card" action: the card is what was asked
  // for, so closing it closes the window rather than revealing the cartridge.
  if (cartridge.memory_card && (await invoke("opens_memory_card").catch(() => false))) {
    await memcard.show(drivePath, closeWindow);
  }
  renderKeys();
  try {
    played = (await invoke("cartridge_stats", { drivePath }))?.games ?? {};
  } catch (error) {
    // An older backend, or a cartridge whose stats file will not read. The
    // sheet is worth drawing either way.
    debugLog(`stats: ${error}`);
    played = {};
  }
  renderSpecs(cartridge);
  renderPaths(cartridge);
  exposePlay(el.card, currentGame());

  // A collection: the cartridge's own name goes above, the rail picks which
  // game the one Play acts on, and the title becomes the selected game.
  if (collected) {
    el.card.classList.add("is-collection");
    // The mark in the corner already prints the collection's name. The eyebrow
    // only stands in when there is no mark to print it — otherwise the window
    // says "Tomb Raider Collection" twice, once as a picture and once as type.
    const marked = !el.cartMark.hidden;
    el.eyebrow.hidden = marked;
    if (!marked) el.eyebrow.textContent = cartridge.title || "Collection";
    el.gameList.hidden = false;
    renderRail(games());
  }

  if (!playable()) {
    el.card.classList.add("is-blocked");
    el.notice.hidden = false;
    el.notice.textContent =
      "No executable set in cartridge.conf, so there is nothing to play. Eject is still available.";
  } else if (nfcSelection) {
    el.notice.hidden = false;
    el.notice.textContent = "READY — press Play to start.";
  }

  const launcherArt = artFor(currentGame());
  if (launcherArt) await showCover(launcherArt);
  else showPlaceholder(currentGame());

  setBusy(false);
  await showWindow();
  await maybeUnbox(launcherArt);
  seat();
  // After the window, deliberately. Copying a save directory takes as long as
  // it takes, and the cartridge should be on screen while it happens rather
  // than the launcher sitting blank behind a file copy nobody can see.
  await syncSaves(drivePath);
  // Only now is there something to point at: with a pad connected the cursor
  // starts on Play.
}

/** The shortcuts, said once in the details sheet, for this kind of cartridge. */
function renderKeys() {
  const keys = ["Enter play", "E eject", "I details"];
  if (isCollection()) keys.splice(1, 0, "1–9 pick and play");
  if (!el.memcardButton.hidden) keys.push("M memory card");
  el.sheetKeys.textContent = `Keys: ${keys.join(" · ")}`;
}

/** What the cartridge remembers, keyed as the stats file keys it. */
let played = {};
/** One line about the saves, once they have been looked at. */
let saveNote = "";

function renderSpecs(info) {
  el.specs.replaceChildren();
  const list = info.games ?? [];
  renderBeat(currentGame() ?? info);
  renderPlayed(info);
  if (saveNote) specRow(el.specs, "Saves", saveNote);
}

/**
 * How often this cartridge has been played, and where.
 *
 * Read from the drive rather than from this machine, which is the only reason
 * it is worth showing: Steam already knows how long you have played on *this*
 * PC. A cartridge with no history yet gets no row, because "0 launches" is a
 * line of furniture rather than a fact anyone wanted.
 */
function renderPlayed(info) {
  const entry = played[keyFor(currentGame()?.executable ?? info.executable ?? "")];
  if (!entry || !entry.launches) return;

  const parts = [];
  if (entry.seconds >= 60) parts.push(duration(entry.seconds));
  parts.push(`${entry.launches} ${entry.launches === 1 ? "launch" : "launches"}`);
  specRow(el.specs, "Played", parts.join(" · "));

  if (entry.lastPlayed) {
    const where = entry.lastHost ? ` on ${entry.lastHost}` : "";
    specRow(el.specs, "Last played", `${since(entry.lastPlayed)}${where}`, true);
  }
}

/** HowLongToBeat's figures, when the cartridge was written with them. */
function renderBeat(game) {
  const beat = game?.how_long ?? {};
  const parts = [];
  if (beat.main) parts.push(`Story ${hoursOf(beat.main)}`);
  if (beat.extra) parts.push(`+Extras ${hoursOf(beat.extra)}`);
  if (beat.complete) parts.push(`100% ${hoursOf(beat.complete)}`);
  if (parts.length) specRow(el.specs, "To beat", parts.join(" · "));
}

/** "27 h", or "40 min" for something short. HowLongToBeat's own rounding. */
function hoursOf(seconds) {
  if (!seconds) return "";
  if (seconds < 3600) return `${Math.round(seconds / 60)} min`;
  const hours = seconds / 3600;
  return `${hours < 10 ? Math.round(hours * 2) / 2 : Math.round(hours)} h`;
}

/**
 * The play numbers for one game, as a skin can reach them.
 *
 * A skin is CSS, and CSS cannot read cartridge.conf, so the launcher reads it
 * and hands every figure over twice: as a custom property (a quoted string for
 * `content:`, a bare number for `calc()`) and, where a skin wants cases rather
 * than a scale, as a data attribute. Set on #card for the game Play will
 * start, and on each .game-row for its own game.
 */
function playFacts(game) {
  const entry = played[keyFor(game?.executable ?? "")] ?? {};
  const beat = game?.how_long ?? {};
  const seconds = entry.seconds ?? 0;
  const progress = beat.main ? Math.min(seconds / beat.main, 1) : null;
  return {
    seconds,
    launches: entry.launches ?? 0,
    lastPlayed: entry.lastPlayed ?? 0,
    beat,
    progress,
  };
}

function progressBand(facts) {
  if (!facts.seconds) return "new";
  if (facts.progress === null) return "unknown";
  if (facts.progress < 0.5) return "started";
  if (facts.progress < 0.9) return "halfway";
  if (facts.progress < 1) return "nearly";
  return "beaten";
}

function exposePlay(node, game) {
  if (!node || !game) return;
  const facts = playFacts(game);
  const quoted = (text) => JSON.stringify(String(text ?? ""));
  const vars = {
    "--playtime": quoted(facts.seconds >= 60 ? duration(facts.seconds) : ""),
    "--playtime-hours": (facts.seconds / 3600).toFixed(2),
    "--launches": String(facts.launches),
    "--last-played": quoted(facts.lastPlayed ? since(facts.lastPlayed) : ""),
    "--hltb-main": quoted(hoursOf(facts.beat.main)),
    "--hltb-extra": quoted(hoursOf(facts.beat.extra)),
    "--hltb-complete": quoted(hoursOf(facts.beat.complete)),
    "--hltb-main-hours": ((facts.beat.main ?? 0) / 3600).toFixed(2),
    "--progress": (facts.progress ?? 0).toFixed(3),
  };
  for (const [name, value] of Object.entries(vars)) node.style.setProperty(name, value);
  node.dataset.played = facts.seconds ? "yes" : "no";
  node.dataset.progress = progressBand(facts);
  node.dataset.hltb = facts.beat.main ? "yes" : "no";

  // The face's own stats line, for the game Play will start.
  if (node === el.card) {
    el.playStatsTime.textContent = facts.seconds >= 60 ? duration(facts.seconds) : "Not played yet";
    el.playStatsBeat.textContent = facts.beat.main ? `of ~${hoursOf(facts.beat.main)}` : "";
    el.playStatsLast.textContent = facts.lastPlayed ? since(facts.lastPlayed) : "";
  }
}

/** Hours and minutes, never seconds: nobody reads a playtime to the second. */
function duration(seconds) {
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.round((seconds % 3600) / 60);
  if (!hours) return `${minutes} min`;
  return minutes ? `${hours} h ${minutes} min` : `${hours} h`;
}

/** A rough distance in the past, which is all anybody wants from this. */
function since(unix) {
  const days = Math.floor((Date.now() / 1000 - unix) / 86400);
  if (days <= 0) return "today";
  if (days === 1) return "yesterday";
  if (days < 30) return `${days} days ago`;
  return new Date(unix * 1000).toLocaleDateString();
}

/**
 * The same key the stats file uses, so a row written on Windows is found on
 * Linux. Kept in step with `stats::key_for` in core: separators normalised,
 * and case folded for URIs only.
 */
function keyFor(executable) {
  const trimmed = String(executable ?? "").trim().replaceAll("\\", "/");
  return /^[a-z][a-z0-9+.-]+:\/\//i.test(trimmed) ? trimmed.toLowerCase() : trimmed;
}

/**
 * Reconcile the saves, and say so only when there is something to say.
 *
 * The backend answers with an empty list when the user has not switched
 * syncing on, so this costs one call and draws nothing in that case. A
 * conflict is the exception: it is the one outcome where the launcher did
 * *not* do the thing the user asked for, and finding that out later — from a
 * save that is three sessions old — is exactly what this feature exists to
 * prevent.
 */
async function syncSaves(drivePath) {
  let slots = [];
  let carriedHome = null;
  try {
    await invoke("sync_saves", { drivePath });
    slots = (await invoke("save_slots", { drivePath })) ?? [];
    carriedHome = await invoke("carried_home", { drivePath });
  } catch (error) {
    debugLog(`saves: ${error}`);
    return;
  }

  // Shader caches, if the cartridge asked to carry them. Fired and not waited
  // on: a warm cache is a gain on the *next* launch, and holding the window for
  // a few hundred megabytes of copying would trade that for a wait now.
  void invoke("pull_shaders", { drivePath })
    .then((carried) => {
      if (!carried?.length) return;
      const bytes = carried.reduce((sum, one) => sum + (one.bytes ?? 0), 0);
      debugLog(`shaders: pulled ${carried.length} caches, ${bytes} bytes`);
    })
    .catch((error) => debugLog(`shaders: ${error}`));

  // A cartridge that carries the game's whole home has no declared slots and is
  // still carrying every save there is. Saying nothing would read as "this
  // cartridge does not do saves", which is the opposite of true.
  if (carriedHome) {
    debugLog(`portable home: ${carriedHome}`);
    saveNote = "the game's own home directory";
    renderSpecs(cartridge);
    if (!slots.length) return;
  }
  if (!slots.length) return;

  const conflicts = slots.filter((slot) => slot.direction === "conflict");
  const carried = slots.filter((slot) => slot.cartridgeBytes > 0).length;
  saveNote = carried ? `${carried} on the cartridge` : "";
  renderSpecs(cartridge);

  if (conflicts.length) {
    const names = conflicts.map((slot) => slot.slot.label).join(", ");
    toast(`${names}: both copies changed, so neither was overwritten.`, true);
  }
}

function showCover(src) {
  return new Promise((resolve) => {
    el.cover.hidden = false;
    el.cover.addEventListener(
      "load",
      () => {
        el.placeholder.classList.add("hidden");
        sampleAccent(el.cover);
        resolve();
      },
      { once: true },
    );
    el.cover.addEventListener("error", () => {
      el.cover.hidden = true;
      resolve();
    }, { once: true });
    el.cover.src = src;
  });
}

/**
 * Wear the look this cartridge brought with it.
 *
 * The launcher used to ship a list of looks and a button to cycle them, and a
 * cartridge picked one by name. Now the cartridge either carries a stylesheet
 * or it does not, which is the arrangement worth keeping: the look belongs to
 * the cartridge, the way the artwork does, so it travels with it.
 *
 * The text goes into a `<style>` rather than an href. Nothing here can name a
 * file, so there is no path on the drive for the window to open — `core` read
 * the stylesheet, capped its size, and handed over the text, which is the same
 * arrangement the artwork already had.
 *
 * Inline rules are live as the assignment returns, so the size and the artwork
 * can be settled immediately; an href would have had to be waited for.
 */
function wearSkin(css = "") {
  debugLog(`wearSkin css=${css.length}b`);
  el.cartSkin.textContent = css;
  resizeToSkin();
  restyleArt();
}

/**
 * Re-derive every picture from the skin that is on now.
 *
 * The three art properties are read while the window is being drawn, so a skin
 * put on afterwards kept the previous one's choices: swapping to a skin that
 * asks for icons left a rail of covers, because the covers had already been
 * decided. Nothing about the cartridge changed, only what to show of it, so
 * this re-reads the properties and redraws rather than reloading anything.
 */
function restyleArt() {
  if (!cartridge) return;

  const hadFocus = document.activeElement?.classList.contains("game-row");
  if (isCollection()) renderRail(games());
  if (hadFocus) {
    el.gameList.querySelector(`.game-row[data-index="${selected}"]`)?.focus();
  }

  const game = currentGame();
  const art = artFor(game);
  if (art) showCover(art);
  else showPlaceholder(game);
  renderLogo();
}

/**
 * Let the skin choose the window's size.
 *
 * The launcher is 420x630 because a cover is 3:4, which is the right shape for
 * one game and the wrong one for a skin laid out sideways. A skin says what it
 * wants in two custom properties and this passes them on; saying nothing keeps
 * the stock size, so this costs a skin that does not care exactly nothing.
 *
 *   :root { --skin-width: 520; --skin-height: 400; }
 */
async function resizeToSkin() {
  if (!tauri?.window) return;

  const style = getComputedStyle(document.documentElement);
  const width = Number.parseInt(style.getPropertyValue("--skin-width"), 10) || 420;
  const height = Number.parseInt(style.getPropertyValue("--skin-height"), 10) || 630;
  if (width === sized.width && height === sized.height) return;
  sized = { width, height };

  try {
    const { getCurrentWindow, LogicalSize } = tauri.window;
    await getCurrentWindow().setSize(new LogicalSize(width, height));
    debugLog(`resized to ${width}x${height}`);
  } catch (error) {
    debugLog(`resize to ${width}x${height} failed: ${error}`);
  }
}

/** What the window was last asked to be, so it is not asked again for nothing. */
let sized = { width: 420, height: 630 };

async function showWindow() {
  if (tauri?.window) await tauri.window.getCurrentWindow().show();

  // Showing is not focusing. The watcher opens this window from the
  // background, and Windows lets a background process put a window in front
  // without giving it the keyboard — so Enter went to whatever was behind it,
  // and a controller reported every button unpressed, because Chromium only
  // tells a focused document what a gamepad is doing.
  try {
    await invoke("focus_window");
  } catch {
    // Nothing to do about it here; the window is up either way.
  }
}

/* ==========================================================================
   Actions
   ========================================================================== */

/** Set while a launch is in flight, so the sweep runs once and Play holds. */
let launching = false;

/**
 * Put Play back to rest.
 *
 * Used to be needed only when a launch failed, because a launch that worked
 * closed the window a moment later and the button went with it. Now the window
 * comes back — so it has to come back saying Play, not still spinning on a
 * launch that finished when the game started an hour ago.
 */
function resetPlay() {
  launching = false;
  el.play.classList.remove("is-launching");
  el.playLabel.textContent = "Play";
  setBusy(false);
}

async function doPlay() {
  const game = currentGame();
  if (!game?.executable || el.play.disabled || launching) return;

  launching = true;
  setBusy(true);
  el.play.classList.add("is-launching");
  el.playLabel.textContent = "Launching";
  toast("Launching…");

  try {
    await invoke("launch_game", {
      executable: game.executable,
      drivePath: cartridge.drive_path,
      // Only decoration in the stats file, but reading it back off the drive
      // would mean parsing the whole cartridge again to get it.
      title: game.title ?? cartridge.title ?? "",
    });
    toast("Launched");
    // The game has the screen now. The launcher steps back to the taskbar
    // rather than closing, so Eject is still there when the game is over.
    setTimeout(standAside, 900);
  } catch (error) {
    toast(String(error), true);
    resetPlay();
  }
}

el.play.addEventListener("click", doPlay);

/**
 * Eject, on the first press.
 *
 * There has never been a blanket confirmation step, and there still is not: a
 * safe eject should cost one press and no paragraph. What the backend does now
 * is look first, and only interrupt when there is something to interrupt about
 * — a game running from the drive, or a file open on it — naming it, and
 * offering to close it. Windows refuses such an eject by itself, with a message
 * naming nothing in particular; this says what to close, and on Linux it is the
 * difference between an eject and a "device is busy" with no culprit in sight.
 *
 * The dialog is native and lives in the backend, so this call simply takes
 * longer while somebody decides. `ejected: false` is the answer when they chose
 * to keep the cartridge where it was, which is not an error and must not slide
 * the cartridge out of the slot.
 */
async function doEject() {
  if (!cartridge || el.eject.disabled) return;

  setBusy(true);
  toast("Ejecting…");
  try {
    const outcome = await invoke("eject_with_guard", { drivePath: cartridge.drive_path });
    if (!outcome?.ejected) {
      toast(outcome?.message || "Left mounted.");
      setBusy(false);
      return;
    }
    // The face leaves the slot, and what is left is the thing to take out —
    // which is the whole message, so the toast stops repeating it.
    dismissToast();
    showSlot(outcome.message || "Safe to remove", null);
    // Long enough to be read by someone across the room reaching for the
    // drive: the words are the confirmation, not the animation.
    setTimeout(closeWindow, SEAT_MS + 2600);
  } catch (error) {
    toast(String(error), true);
    setBusy(false);
  }
}

el.eject.addEventListener("click", doEject);

// A controller reaches the same four things the keyboard does, and nothing
// more: it moves focus and clicks, so it cannot start anything a person at the
// keyboard could not.
/**
 * Write a line to the launcher log, when there is one.
 *
 * A no-op unless PC_GAMEPAK_DEBUG is set, which is decided once below rather
 * than on every call — a gamepad poll runs sixty times a second and is not
 * somewhere to put a round trip.
 */
let debugOn = null;
let debugPending = [];
function debugLog(line) {
  const stamped = `[${new Date().toISOString()}] ${line}`;
  // Held until the answer arrives rather than dropped. Whether logging is on is
  // one round trip, and the most interesting lines — what the cartridge asked
  // for, what the window resized to — all happen inside it.
  if (debugOn === null) debugPending.push(stamped);
  else if (debugOn) invoke("debug_log", { line: stamped });
}

invoke("debug_logging")
  .then((on) => {
    debugOn = Boolean(on);
    if (debugOn) {
      invoke("debug_log", { line: `[${new Date().toISOString()}] launcher started` });
      for (const line of debugPending) invoke("debug_log", { line });
    }
    debugPending = [];
  })
  .catch(() => {
    debugOn = false;
    debugPending = [];
  });

/* ==========================================================================
   Memory card
   ========================================================================== */

/** The drive this window is for, once init has it. */
let memcardDrive = "";

const memcard = createMemoryCard({ invoke, toast, formatBytes, duration, since });

/** On a combo drive, swap between the cartridge and its memory card. */
async function toggleMemoryCard() {
  if (memcard.open) {
    memcard.hide();
    return;
  }
  if (el.memcardButton.hidden || !memcardDrive) return;
  try {
    await memcard.show(memcardDrive, () => memcard.hide(), { back: true });
  } catch (error) {
    toast(String(error), true);
  }
}

el.memcardButton.addEventListener("click", toggleMemoryCard);

// While the card is open its keys are its own: Enter copies a save rather than
// pressing Play on the cartridge underneath.
window.addEventListener(
  "keydown",
  (event) => {
    if (unboxing || event.metaKey || event.ctrlKey || event.altKey) return;
    if (memcard.open) {
      if (memcard.key(event)) event.stopImmediatePropagation();
      return;
    }
    if ((event.key === "m" || event.key === "M") && !el.memcardButton.hidden) {
      event.preventDefault();
      event.stopImmediatePropagation();
      void toggleMemoryCard();
    }
  },
  { capture: true },
);

/**
 * Where a pad button goes: skipping the unboxing, then the memory card if it
 * is open, then the launcher.
 */
const route = (action, onCard) => (...args) => {
  if (unboxing) return unboxing();
  if (memcard.open) return onCard?.(...args);
  return action(...args);
};

const gamepad = connectGamepad({
  play: route(doPlay, memcard.pad.primary),
  // X ejects the cartridge and does nothing on the card: a button that ejects
  // on one screen must not delete on the next. Delete is Y there, pressed twice.
  eject: route(doEject, () => {}),
  details: route(() => toggleSheet(), memcard.pad.remove),
  // View opens and closes the memory card on a combo drive.
  card: route(
    () => void toggleMemoryCard(),
    () => {
      if (!el.memcardButton.hidden) void toggleMemoryCard();
    },
  ),
  move: route((x, y) => move(x, y), memcard.pad.move),
  log: debugLog,
  back: route(() => {
    if (el.sheet.classList.contains("is-open")) toggleSheet(false);
    else closeWindow();
  }, memcard.pad.back),
});

el.close.addEventListener("click", closeWindow);
el.details.addEventListener("click", () => toggleSheet());
el.sheetClose.addEventListener("click", () => toggleSheet(false));
el.pathsToggle.addEventListener("click", () => togglePaths());
// One way through to the wizard, which is where both making a cartridge and
// changing a setting happen. Two links here asked the reader to know which of
// them opened which half of the same window.
el.openWizard.addEventListener("click", async () => {
  try {
    await invoke("open_wizard_window");
  } catch (error) {
    console.error(error);
  }
});

document.addEventListener("keydown", (event) => {
  if (event.metaKey || event.ctrlKey || event.altKey) return;
  const sheetOpen = el.sheet.classList.contains("is-open");

  // 1-9 start the nth game of a collection, so a cartridge you know can be
  // played without reaching for the mouse or arrowing down the rail. The
  // selection follows, so the window shows what it just started.
  if (!sheetOpen && isCollection() && /^[1-9]$/.test(event.key)) {
    const index = Number(event.key) - 1;
    if (index < games().length) {
      event.preventDefault();
      select(index);
      doPlay();
    }
    return;
  }

  // The arrows move the selection, which is what the rail is for. They used to
  // scroll it, so a collection could be scrolled past a game without ever
  // choosing one — and Play still acted on whatever was selected before.
  //
  // All four of them, through the same two-axis step the pad uses: left and
  // right did nothing at all in the skins whose list runs sideways or wraps
  // into a grid, which are the two layouts where they are the natural keys.
  if (!sheetOpen && isCollection() && ARROWS[event.key]) {
    event.preventDefault();
    const [x, y] = ARROWS[event.key];
    move(x, y);
    return;
  }

  switch (event.key) {
    case "Enter":
      if (sheetOpen) return;
      event.preventDefault();
      doPlay();
      break;
    case "e":
    case "E":
      event.preventDefault();
      el.eject.click();
      break;
    case "i":
    case "I":
      event.preventDefault();
      toggleSheet();
      break;
    case "Escape":
      event.preventDefault();
      // Escape peels one layer at a time: a waiting error, then the sheet, then
      // the window itself.
      if (el.toast.classList.contains("is-sticky")) dismissToast();
      else if (sheetOpen) toggleSheet(false);
      else closeWindow();
      break;
  }
});

/* ==========================================================================
   Browser preview
   --------------------------------------------------------------------------
   Opened outside Tauri, the page serves a sample cartridge so the window can
   be designed and reviewed without a physical drive:
       python -m http.server 8731   (from the repository root)
       localhost:8731/tauri-ui/app/index.html?drive=D:\
   Append &skin=<name> to wear one of docs/skins/ as if the cartridge carried
   it, which is the only way a look reaches the launcher.
   Append &state=noexec to see the nothing-to-play case, &state=bundle for a
   collection, and &fixed=1 for a cartridge on a path rather than a drive —
   which composes with the others.
   ========================================================================== */

/**
 * A logo for the preview, as a `data:` URI because that is what the backend
 * sends and what `renderTitle` will accept — a path is refused there, so a
 * skin asking for `--skin-logo: game` would show nothing without this.
 */
const DEMO_LOGO =
  "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg'" +
  " viewBox='0 0 300 80'%3E%3Ctext x='0' y='58' font-family='Georgia'" +
  " font-size='54' fill='%23fff'%3EDEMO LOGO%3C/text%3E%3C/svg%3E";

/**
 * The stylesheet the preview's cartridge is carrying, if the URL named one.
 *
 * `?skin=retro` fetches `docs/skins/retro.css` and hands it over as the
 * cartridge's own `skin_css`, which is the only way a look reaches the launcher
 * now — so the preview exercises the real path rather than a special case.
 * Needs the server rooted at the repository, not at `tauri-ui/app`.
 */
async function demoSkinCss() {
  const name = new URLSearchParams(location.search).get("skin");
  if (!name || !/^[a-z0-9-]+$/.test(name)) return "";
  try {
    const response = await fetch(`../../docs/skins/${name}.css`);
    return response.ok ? await response.text() : "";
  } catch {
    return "";
  }
}

/** The preview's memory card: one save in every state worth drawing. */
const demoBlocks = (() => {
  const now = Math.floor(Date.now() / 1000);
  const block = (id, title, icon, direction, hostBytes, cartridgeBytes, seconds, ago) => ({
    slot: { id, label: title, template: `{appdata}/${id}`, mode: "copy" },
    hostPath: direction === "unusable" ? "" : `C:\\Users\\you\\AppData\\Roaming\\${id}`,
    direction,
    hostNewest: now - ago,
    cartridgeNewest: now - ago - 3600,
    hostBytes,
    cartridgeBytes,
    lastSync: 0,
    detail: direction === "unusable" ? "This PC has no folder for it" : "",
    title,
    icon,
    seconds,
    launches: 3,
  });
  return [
    block("stardew", "Stardew Valley", "src/demo/cover.jpg", "inSync", 2_400_000, 2_400_000, 187_200, 86_400 * 2),
    block("gow", "God of War", "src/demo/gow-1.jpg", "pull", 31_000_000, 33_500_000, 47_520, 86_400),
    block("ragnarok", "God of War Ragnarök", "src/demo/gow-2.jpg", "push", 41_000_000, 38_000_000, 21_000, 3_600),
    block("tunic", "Tunic", "", "conflict", 800_000, 810_000, 9_000, 86_400 * 6),
    block("bluey", "Bluey: The Videogame", "src/demo/gow-collection.jpg", "pull", 0, 120_000, 3_600, 86_400 * 12),
    block("celeste", "Celeste", "", "unusable", 0, 96_000, 0, 86_400 * 30),
  ];
})();

async function demoInvoke(command, args) {
  const state = new URLSearchParams(location.search).get("state");
  switch (command) {
    case "parse_cartridge":
      if (state === "bundle") {
        return {
          skin_css: await demoSkinCss(),
          title: "God of War Collection",
          cover: "src/demo/gow-collection.jpg",
          cover_path: "D:\\collection.jpg",
          executable: "steam://rungameid/310970",
          drive_path: args.drivePath,
          is_bundle: true,
          holds_game: true,
          // All four kinds filled in, and each one different, because the
          // three art properties can ask for a different one in each of the
          // three places a picture goes and there is no way to see that they
          // are being honoured if every kind is the same file.
          games: [
            {
              title: "God of War (2018)",
              executable: "steam://rungameid/310970",
              cover: "src/demo/gow-1.jpg",
              background: "src/demo/gow-2.jpg",
              logo: DEMO_LOGO,
              icon: "src/demo/cover.jpg",
              cover_path: "",
              sizeBytes: 64_200_000_000,
              how_long: { main: 75_600, extra: 118_800, complete: 183_600 },
            },
            {
              title: "God of War: Ragnarök",
              executable: "steam://rungameid/1476670",
              cover: "src/demo/gow-2.jpg",
              background: "src/demo/gow-1.jpg",
              logo: DEMO_LOGO,
              icon: "src/demo/cover.jpg",
              cover_path: "",
              sizeBytes: 44_700_000_000,
            },
          ],
        };
      }
      return {
        skin_css: await demoSkinCss(),
        title: "Cinder & Salt",
        // A plain path stands in for the data URI the backend would send; the
        // browser loads it directly.
        cover: "src/demo/cover.jpg",
        cover_path: "D:\\cover.jpg",
        executable: state === "noexec" ? "" : "steam://rungameid/367520",
        drive_path: args.drivePath,
        is_bundle: false,
        holds_game: true,
        games: [],
        memory_card: new URLSearchParams(location.search).has("combo"),
      };
    case "can_eject":
      // A cartridge on a fixed path has no drive to unmount, so the button
      // should not be there.
      return !new URLSearchParams(location.search).has("fixed");
    case "cartridge_stats":
      // A cartridge that has been somewhere: the preview is the only place
      // the Played rows can be looked at without a drive in hand.
      return {
        version: 1,
        games: {
          "steam://rungameid/310970": {
            title: "God of War (2018)",
            launches: 9,
            seconds: 47_520,
            firstPlayed: Math.floor(Date.now() / 1000) - 86_400 * 40,
            lastPlayed: Math.floor(Date.now() / 1000) - 86_400 * 2,
            lastHost: "deck",
            sessions: [
              { started: Math.floor(Date.now() / 1000) - 86_400 * 9, seconds: 7_200, host: "workshop" },
              { started: Math.floor(Date.now() / 1000) - 86_400 * 5, seconds: 5_400, host: "deck" },
              { started: Math.floor(Date.now() / 1000) - 86_400 * 2, seconds: 3_900, host: "deck" },
            ],
          },
          "steam://rungameid/1091500": {
            title: "Cyberpunk 2077",
            launches: 3,
            seconds: 9_300,
            firstPlayed: Math.floor(Date.now() / 1000) - 86_400 * 9,
            lastPlayed: Math.floor(Date.now() / 1000) - 86_400,
            lastHost: "workshop",
          },
        },
      };
    case "eject_with_guard":
      return { ejected: true, message: "Safe to remove" };
    case "cartridge_busy":
      return { holders: [], unchecked: 0 };
    case "sync_saves":
      return [];
    case "carried_home":
      return null;
    case "pull_shaders":
    case "push_shaders":
      return [];
    case "shader_slots":
      return [];
    // `?state=memcard` is a memory card; `&combo` makes a cartridge one too.
    // `&saves=N` shows N saves (the six repeated), `&saves=error` an unreadable
    // card, for looking at a card emptier or fuller than the six.
    case "memory_card": {
      const saves = new URLSearchParams(location.search).get("saves");
      if (saves === "error") throw "the drive stopped answering";
      if (saves !== null) {
        const count = Number(saves);
        const many = Array.from({ length: count }, (_, i) => {
          const base = demoBlocks[i % demoBlocks.length];
          return i < demoBlocks.length
            ? base
            : { ...base, slot: { ...base.slot, id: `${base.slot.id}-${i}` }, title: `${base.title} ${Math.floor(i / demoBlocks.length) + 1}` };
        });
        demoBlocks.splice(0, demoBlocks.length, ...many);
      }
      return {
        title: state === "memcard" ? "Harry's Memory Card" : "Cinder & Salt",
        cardOnly: state === "memcard",
        blocks: demoBlocks,
      };
    }
    case "memcard_copy": {
      const block = demoBlocks.find((b) => b.slot.id === args.slotId);
      if (block) Object.assign(block, { direction: "inSync", hostBytes: block.cartridgeBytes || block.hostBytes, cartridgeBytes: block.cartridgeBytes || block.hostBytes });
      return { backup: "kept" };
    }
    case "memcard_reveal":
      console.log("[preview] open save folder", args.slotId);
      return null;
    case "memcard_remove": {
      const block = demoBlocks.find((b) => b.slot.id === args.slotId);
      if (block) Object.assign(block, { cartridgeBytes: 0, direction: block.hostBytes ? "push" : "empty" });
      return null;
    }
    case "save_slots":
      return [
        {
          slot: { id: "saves", label: "Saves", template: "{appdata}/Foo/Saves", mode: "copy" },
          hostPath: "/home/you/.config/Foo/Saves",
          direction: "inSync",
          hostNewest: 0,
          cartridgeNewest: 0,
          hostBytes: 4096,
          cartridgeBytes: 4096,
          lastSync: 0,
          detail: "",
        },
      ];
    case "cartridge_health":
      // The preview shows the case worth designing for: a link that is fine,
      // a transport that is not, and a drive with no room left.
      return {
        link: "10 Gbps",
        linkMbps: 10000,
        transport: "BOT",
        label: "CINDER",
        filesystem: "exFAT",
        totalBytes: 128_035_676_160,
        freeBytes: 9_663_676_416,
        usedPercent: 92,
        warnings: [
          "Running in BOT mode, which sends one command at a time. UASP queues them and is worth roughly two to three times as much on the small random reads a game streams. Usually a different port or enclosure firmware fixes it.",
          "92% full. These drives have no DRAM of their own and cannot borrow host memory over USB, so the last 15% costs more than it looks like it should. Leaving some room back keeps random reads quick.",
        ],
      };
    default:
      console.log("[preview]", command, args);
      return "";
  }
}

init();