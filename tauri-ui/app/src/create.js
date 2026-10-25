/**
 * PC GamePak — create.js
 *
 * The wizard. Selection is always multiple: ticking one game builds a
 * cartridge, ticking three builds a multicartridge, and the rail on the right
 * titles itself to match. There is no bundle mode to enter and no + to find.
 *
 * The left column moves through phases — the library, then a name and a face
 * if there is more than one game, then what to put on it, then the write
 * itself. The rail never changes place: it always says what is being built.
 *
 * Backend contract (src-tauri/src/main.rs):
 *   list_games()                     -> { games, problems }
 *   list_target_drives()             -> [{ path, label, totalBytes, freeBytes, hasCartridge }]
 *   list_unmounted_volumes()         -> [{ disk, partition, label, filesystem, totalBytes }]
 *   mount_volume({ volume })         -> the new drive root
 *   game_cover({ library, id })      -> data URI or ""
 *   executable_choices({ ... })      -> [{ relative, name, score }]
 *   format_plan({ drivePath })       -> { currentLabel, device, totalBytes, warning }
 *   steam_registration({ drivePath })-> bool
 *   holds_steam_games({ drivePath })  -> bool
 *   steam_registration_plan({ drivePath }) -> [string]
 *   register_with_steam({ drivePath })-> bool
 *   suggest_collection_name({ titles }) -> string
 *   pick_cover_image()               -> { path, preview } | null
 *   pick_game_folder()               -> { path, name, sizeBytes, choices } | null
 *   add_gamepak(title, kind, a, b, id) -> "gp_…" (generic action wizard)
 *   registered_gamepaks()              -> [{ id, title, action, triggers }]
 *   remove_gamepak(id)                 -> ()
 *   moonlight_apps(host)               -> [app name] (`moonlight list <host>`)
 *   moonlight_hosts()                  -> [{ name, apps }] (hosts paired in Moonlight)
 *   remove_gamepak_trigger(id, trigger) -> ()
 *   last_trigger()                     -> [id, ms] | null (what Learn waits on)
 *   add_gamepak_trigger(id, trigger)   -> ()
 *   create_cartridge({ request })    -> { confPath, formatted, gameCopied, ... }
 *
 * The backend re-derives the list of writable drives and re-checks the format
 * confirmation itself, so nothing here can talk it into writing to the wrong
 * place.
 */

const tauri = window.__TAURI__;
const invoke = tauri?.core?.invoke ?? demoInvoke;

const $ = (id) => document.getElementById(id);

const el = {
  shell: $("shell"),
  barText: $("bar-text"),
  settings: $("btn-settings"),
  close: $("btn-close"),

  phases: {
    build: $("step-build"),
    custom: $("step-custom"),
    running: $("step-running"),
  },
  search: $("search"),
  games: $("games"),
  gamesEmpty: $("games-empty"),
  btnCustom: $("btn-custom"),

  // By hand
  customCover: $("custom-cover"),
  btnCustomCover: $("btn-custom-cover"),
  btnPickFolder: $("btn-pick-folder"),
  btnClearFolder: $("btn-clear-folder"),
  folderChosen: $("folder-chosen"),
  btnPickRom: $("btn-pick-rom"),
  btnClearRom: $("btn-clear-rom"),
  romChosen: $("rom-chosen"),
  customTitle: $("custom-title"),
  titleHint: $("title-hint"),
  exeChoices: $("exe-choices"),
  exeHint: $("exe-hint"),
  customExec: $("custom-exec"),
  btnCustomBack: $("btn-custom-back"),
  btnCustomDone: $("btn-custom-done"),

  // Name it
  collectionCover: $("collection-cover"),
  collectionCoverImg: $("collection-cover-img"),
  btnInheritCover: $("btn-inherit-cover"),
  collectionTitle: $("collection-title"),
  orderList: $("order-list"),

  // Options

  // Running
  runningPhase: $("running-phase"),
  runningTitle: $("running-title"),
  writtenDone: $("written-done"),
  writtenTotal: $("written-total"),
  remaining: $("remaining"),
  progressTrack: $("progress-track"),
  progressFill: $("progress-fill"),
  rate: $("rate"),
  currentFile: $("current-file"),
  log: $("log"),

  // Rail
  railEmpty: $("rail-empty"),
  railBody: $("rail-body"),
  railKindText: $("rail-kind-text"),
  railKindCount: $("rail-kind-count"),
  railLauncherCard: $("rail-launcher-card"),
  railLauncherArt: $("rail-launcher-art"),
  railLauncherScrim: $("rail-launcher-scrim"),
  railLauncherStage: $("rail-launcher-stage"),
  railLauncherEyebrow: $("rail-launcher-eyebrow"),
  railLauncherLogo: $("rail-launcher-logo"),
  railLauncherTitle: $("rail-launcher-title"),
  railExplorer: $("rail-explorer"),
  railExplorerLabel: $("rail-explorer-label"),
  railExplorerIcon: $("rail-explorer-icon"),
  railExplorerName: $("rail-explorer-name"),
  railExplorerFree: $("rail-explorer-free"),
  railSpace: $("rail-space"),
  railDriveLabel: $("rail-drive-label"),
  railDriveFill: $("rail-drive-fill"),
  railBar: $("rail-bar"),
  drives: $("drives"),
  drivesEmpty: $("drives-empty"),
  btnRegister: $("btn-register"),
  btnUnregister: $("btn-unregister"),
  moveRow: $("move-row"),
  optMove: $("opt-move"),
  moveHint: $("move-hint"),
  railPlan: $("rail-plan"),
  plan: $("plan"),
  railForced: $("rail-forced"),
  forced: $("forced"),
  readyMessage: $("ready-message"),
  create: $("btn-create"),
  createLabel: document.querySelector("#btn-create .btn__label"),
  status: $("status"),

  // Dialogs
  columns: $("columns"),
  tabCreate: $("tab-create"),
  tabEdit: $("tab-edit"),
  tabCreated: $("tab-created"),
  tabMemcard: $("tab-memcard"),
  panelMemcard: $("panel-memcard"),
  tabActions: $("tab-actions"),
  panelActions: $("panel-actions"),
  gpHome: $("gp-home"),
  gpNew: $("gp-new"),
  gpList: $("gp-list"),
  gpEmpty: $("gp-empty"),
  gpStatus: $("gp-status"),
  gpForm: $("gp-form"),
  gpListen: $("gp-listen"),
  gpListenText: $("gp-listen-text"),
  gpTypeId: $("gp-type-id"),
  gpIdField: $("gp-id-field"),
  gpId: $("gp-id"),
  gpKinds: $("gp-kinds"),
  gpGame: $("gp-game"),
  gpGameSearch: $("gp-game-search"),
  gpGameResults: $("gp-game-results"),
  gpFields: $("gp-fields"),
  gpA: $("gp-a"),
  gpALabel: $("gp-a-label"),
  gpB: $("gp-b"),
  gpBField: $("gp-b-field"),
  gpBLabel: $("gp-b-label"),
  gpAList: $("gp-a-list"),
  gpMl: $("gp-ml"),
  gpMlFind: $("gp-ml-find"),
  gpMlSearch: $("gp-ml-search"),
  gpMlResults: $("gp-ml-results"),
  gpTitle: $("gp-title"),
  gpFormStatus: $("gp-form-status"),
  gpCancel: $("gp-cancel"),
  gpSave: $("gp-save"),
  optMemcard: $("opt-memcard"),
  editMemcard: $("edit-memcard"),
  optPlatform: $("opt-platform"),
  editPlatform: $("edit-platform"),
  setLudusavi: $("set-ludusavi"),
  mcwDrive: $("mcw-drive"),
  mcwDriveNote: $("mcw-drive-note"),
  mcwName: $("mcw-name"),
  mcwGames: $("mcw-games"),
  mcwGamesEmpty: $("mcw-games-empty"),
  mcwSearch: $("mcw-search"),
  mcwResults: $("mcw-results"),
  mcwStatus: $("mcw-status"),
  mcwWrite: $("btn-mcw-write"),
  panelCreate: $("panel-create"),
  panelEdit: $("panel-edit"),
  panelCreated: $("panel-created"),
  createdList: $("created-list"),
  createdEmpty: $("created-empty"),
  createdCount: $("created-count"),
  editMediaTitle: $("edit-media-title"),
  editMediaMeta: $("edit-media-meta"),
  btnChangeCart: $("btn-change-cart"),
  editFormWrap: $("edit-form-wrap"),
  btnRefetchArt: $("btn-refetch-art"),
  btnRemoveLogo: $("btn-remove-logo"),
  editTitle: $("edit-title"),
  editArtSlots: $("edit-art-slots"),
  editCoverSlot: $("edit-cover-slot"),
  editCover: $("edit-cover"),
  editCoverImg: $("edit-cover-img"),
  editLogo: $("edit-logo"),
  editLogoImg: $("edit-logo-img"),
  editIcon: $("edit-icon"),
  editIconImg: $("edit-icon-img"),
  editHero: $("edit-hero"),
  editHeroImg: $("edit-hero-img"),
  editGames: $("edit-games"),
  editStatus: $("edit-status"),
  tuneNow: $("tune-now"),
  tunePlan: $("tune-plan"),
  tuneStatus: $("tune-status"),
  btnTune: $("btn-tune"),
  btnUntune: $("btn-untune"),
  btnTuneRun: $("btn-tune-run"),
  btnTuneCancel: $("btn-tune-cancel"),

  changeGame: $("btn-change-game"),
  changeMedia: $("btn-change-media"),
  pickDialog: $("pick-dialog"),
  pickForm: $("pick-form"),
  pickTitle: $("pick-title"),
  tickCount: $("tick-count"),
  pickGames: $("pick-games"),
  pickMedia: $("pick-media"),

  buildGamePlate: $("build-game-plate"),
  buildGameCover: $("build-game-cover"),
  buildGameTitle: $("build-game-title"),
  buildGameMeta: $("build-game-meta"),
  buildName: $("build-name"),
  buildMediaTitle: $("build-media-title"),
  buildMediaMeta: $("build-media-meta"),
  artSlots: $("art-slots"),
  coverSlot: $("cover-slot"),
  slotLogo: $("slot-logo"),
  slotLogoImg: $("slot-logo-img"),
  slotIcon: $("slot-icon"),
  slotHero: $("slot-hero"),
  slotHeroImg: $("slot-hero-img"),
  slotIconImg: $("slot-icon-img"),

  optCopy: $("opt-copy"),
  optCopyRow: $("opt-copy-row"),
  optCopyMeta: $("opt-copy-meta"),
  optVerify: $("opt-verify"),
  optVerifyRow: $("opt-verify-row"),
  optEject: $("opt-eject"),
  optFormat: $("opt-format"),
  optFormatRow: $("opt-format-row"),
  optFilesystem: $("opt-filesystem"),

  confirmDialog: $("confirm-dialog"),
  confirmForm: $("confirm-form"),
  confirmTitle: $("confirm-title"),
  confirmText: $("confirm-text"),

  settingsDialog: $("settings-dialog"),
  sources: $("sources"),
  scanAge: $("scan-age"),
  btnRescan: $("btn-rescan"),
  setSgdb: $("set-sgdb"),
  sgdbKeyField: $("sgdb-key-field"),
  setSgdbKey: $("set-sgdb-key"),
  setFilesystem: $("set-filesystem"),
  setFilesystemNote: $("set-filesystem-note"),
  setCopy: $("set-copy"),
  setVerify: $("set-verify"),
  setIcon: $("set-icon"),
  setEject: $("set-eject"),
  setCloseSteam: $("set-close-steam"),
  setTune: $("set-tune"),
  setTuneRow: $("set-tune-row"),
  setTrim: $("set-trim"),
  setTrimRow: $("set-trim-row"),
  setCopyRate: $("set-copy-rate"),
  frontends: $("frontends"),
  frontendsWarning: $("frontends-warning"),
  setOnInsert: $("set-on-insert"),
  onInsertHint: $("on-insert-hint"),
  setPlaytime: $("set-playtime"),
  setLaunchOnTrigger: $("set-launch-on-trigger"),
  setIdle: $("set-idle"),
  setIdleRow: $("set-idle-row"),
  setHltb: $("set-hltb"),
  setSaveSync: $("set-save-sync"),
  settingsSave: $("settings-save"),
  settingsStatus: $("settings-status"),

  sgdbDialog: $("sgdb-dialog"),
  sgdbForm: $("sgdb-form"),
  sgdbTitle: $("sgdb-title"),
  sgdbTabs: $("sgdb-tabs"),
  sgdbSearch: $("sgdb-search"),
  sgdbStatus: $("sgdb-status"),
  sgdbResults: $("sgdb-results"),
  icoReceipt: $("ico-receipt"),
  sgdbManualUrl: $("sgdb-manual-url"),
  sgdbUseManual: $("sgdb-use-manual"),
  sgdbUseLocal: $("sgdb-use-local"),
  previewArt: $("preview-art"),
  previewGrid: $("preview-grid"),
  previewStage: $("preview-stage"),
  previewLogo: $("preview-logo"),
  previewTitle: $("preview-title"),
  previewIcon: $("preview-icon"),
  previewIconLabel: $("preview-icon-label"),
  previewHeroImg: $("preview-hero-img"),
  sgdbPreview: $("sgdb-preview"),
};

/* ==========================================================================
   State
   ========================================================================== */

let library = [];
let drives = [];
// Volumes Windows can read but has not lettered. Not drives yet — nothing
// can be written to a volume with no mount point — so they are held apart
// from `drives` and only join it once one has been assigned.
let unmounted = [];
/** Ticked games, in the order they were ticked — which is the play order. */
let picked = [];
let selectedDrive = null;
let formatPlan = null;
let building = false;
let platform = "";
let driveIsSteamLibrary = false;
/** Whether the drive carries Steam games, registered or not. */
let driveHoldsSteamGames = false;
/** What registering would change, once it has been asked for and shown. */
let steamPlan = [];
let scannedAt = null;

/** Artwork chosen by hand, per role. */
const art = { cover: null, logo: null, icon: null, background: null };

/** The by-hand game, when one is being entered. */
let manual = null;

let settings = {
  steamgriddbEnabled: false,
  steamgriddbApiKey: "",
  defaultFilesystem: "ntfs",
  defaultVerify: true,
  defaultIcon: true,
  defaultEject: true,
};

let editing = null;
let activeTab = "create";

/**
 * The choices made for this one cartridge.
 *
 * Settings holds the defaults; these start from them and are what the Options
 * toggles on the build screen change. They used to be Settings and nothing
 * else, which made "erase this drive" a preference that stayed switched on for
 * every cartridge after it. Erasing always starts off.
 */
const run = {
  copy: true,
  verify: true,
  eject: true,
  format: false,
  filesystem: "ntfs",
};

/** Put the per-cartridge choices back to what Settings says. */
function resetRunOptions() {
  run.copy = settings.defaultCopy !== false;
  run.verify = settings.defaultVerify !== false;
  run.eject = settings.defaultEject !== false;
  run.format = false;
  run.filesystem = filesystemInfo(settings.defaultFilesystem)
    ? settings.defaultFilesystem
    : DEFAULT_FILESYSTEM;
}

/** Set once a write has finished, until the next cartridge is started. */
let finished = false;

/**
 * Move rather than copy: delete the PC's Steam install once the cartridge's copy
 * has verified. Run-state like `run.copy`, but never seeded from Settings —
 * a default that deletes games is not a default anyone should inherit.
 */
let moveWanted = false;

/** How many of the chosen games came out of Steam. */
function steamGames() {
  return picked.filter((g) => g.library === "steam").length;
}

/** The Steam games a move would take off this PC. */
function movable() {
  return copyable().filter((g) => g.library === "steam");
}

const on = {
  get copy() { return run.copy && copyable().length > 0; },
  get verify() { return run.verify; },
  // The backend refuses a move without a verified copy; this refuses first.
  get move() { return moveWanted && this.copy && this.verify && movable().length > 0; },
  get icon() { return settings.defaultIcon !== false; },
  get eject() { return run.eject; },
  // Only meaningful when Steam is involved: it exists so Steam does not write
  // its library list back over the cartridge on exit.
  get closeSteam() {
    return settings.defaultCloseSteam !== false && steamGames() > 0;
  },
  get tune() { return platform === "windows" && Boolean(settings.defaultTune); },
  // Formatting already discards the whole volume, and Windows does this on its
  // own schedule, so neither case asks.
  get trim() {
    return Boolean(settings.defaultTrim) && !this.format && platform !== "windows";
  },
  get format() { return run.format && Boolean(selectedDrive); },
};

let phase = "build";

/* ==========================================================================
   Small helpers
   ========================================================================== */

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

/** "about 11 minutes" — never a false precision, and never a bare number. */
function formatDuration(seconds) {
  if (!Number.isFinite(seconds) || seconds <= 0) return "";
  if (seconds < 90) return `about ${Math.max(10, Math.round(seconds / 10) * 10)} s`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `about ${minutes} minute${minutes === 1 ? "" : "s"}`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest ? `about ${hours} h ${rest} min` : `about ${hours} hour${hours === 1 ? "" : "s"}`;
}

/** mm:ss, for a countdown that is being watched. */
function clock(seconds) {
  if (!Number.isFinite(seconds) || seconds < 0) return "—";
  const m = Math.floor(seconds / 60);
  const s = Math.round(seconds % 60);
  return `${m}:${String(s).padStart(2, "0")}`;
}

function status(message, kind = "") {
  el.status.textContent = message;
  el.status.className = kind ? `selectable is-${kind}` : "selectable";
}

function safeSrc(src) {
  const value = String(src ?? "");
  // A cover arrives as a data: URI from Rust, or a relative path in the browser
  // preview. Nothing else may reach an img src.
  if (value.startsWith("data:image/")) return value;
  if (/^[\w./-]+$/.test(value) && !value.startsWith("//")) return value;
  return "";
}

/** The only hosts the CSP will load an image from. */
const ARTWORK_HOST = /^([\w-]+\.)*steamgriddb\.com$/;

/**
 * A thumbnail in the artwork picker, which is the one remote image on the page.
 *
 * Deliberately not part of `safeSrc`: everything else here is a data: URI from
 * Rust or a file sitting beside this one, and widening that guard to let an
 * https URL through would let one into every img on the page. The picker sent
 * its thumbnails through `safeSrc` anyway, which refused every one of them —
 * so the results grid was a wall of blank tiles with only "600×900" underneath,
 * and the artwork was picked without ever being seen.
 */
function safeRemoteSrc(src) {
  let url;
  try {
    url = new URL(String(src ?? ""));
  } catch {
    return "";
  }
  return url.protocol === "https:" && ARTWORK_HOST.test(url.hostname) ? url.href : "";
}

/** Put art into a .plate, or take it away. */
function setPlate(plate, img, src) {
  const safe = safeSrc(src);
  if (safe) {
    img.src = safe;
    img.hidden = false;
    plate.classList.add("has-art");
  } else {
    img.hidden = true;
    img.removeAttribute("src");
    plate.classList.remove("has-art");
  }
}

/* ==========================================================================
   Phases
   ========================================================================== */

function showPhase(next) {
  phase = next;
  for (const [name, node] of Object.entries(el.phases)) {
    node.hidden = name !== next;
  }
  el.shell.classList.toggle("is-running", next === "running");
  refreshRail();
}

/**
 * Whether this is one game or several.
 *
 * The whole selection model is this predicate: a cartridge takes its name and
 * art from the game itself, a multicartridge has neither and has to be given
 * both, which is what step 2 exists for.
 */
function isCollection() {
  return picked.length > 1;
}

function totalBytes() {
  return picked.reduce((sum, g) => sum + (g.sizeOnDisk || 0), 0);
}

/* ==========================================================================
   Step 1 — the library
   ========================================================================== */

function matches(game, query) {
  if (!query) return true;
  return game.name.toLowerCase().includes(query) ||
    String(game.source ?? "").toLowerCase().includes(query);
}

function renderGames() {
  const query = el.search.value.trim().toLowerCase();
  const shown = library.filter((g) => matches(g, query));

  el.games.replaceChildren();
  for (const game of shown) {
    const li = document.createElement("li");
    const label = document.createElement("label");
    label.className = "game";
    label.classList.toggle("is-on", picked.some((p) => p.id === game.id));

    const box = document.createElement("input");
    box.type = "checkbox";
    box.checked = picked.some((p) => p.id === game.id);
    box.addEventListener("change", () => toggle(game, box.checked));

    const name = document.createElement("span");
    name.className = "game__name";
    name.textContent = game.name;

    const meta = document.createElement("span");
    meta.className = "game__meta";
    const size = document.createElement("span");
    if (game.sizeOnDisk) {
      size.textContent = formatBytes(game.sizeOnDisk);
    } else {
      // Not installed locally, so there is no size to report and nothing to
      // copy — the cartridge would be a key to an installed copy.
      size.textContent = "—";
      size.className = "is-unknown";
    }
    const divider = document.createElement("span");
    divider.className = "game__divider";
    const source = document.createElement("span");
    source.textContent = game.source || game.library || "";
    meta.append(size, divider, source);

    label.append(box, name, meta);
    li.append(label);
    el.games.append(li);
  }

  el.gamesEmpty.hidden = shown.length > 0;
  if (shown.length === 0) {
    el.gamesEmpty.textContent = query
      ? "Nothing in your library matches that."
      : "No games found. Add one by hand below.";
  }
}

function toggle(game, on) {
  if (on) {
    if (!picked.some((p) => p.id === game.id)) picked.push(game);
  } else {
    picked = picked.filter((p) => p.id !== game.id);
  }
  // Ticking a real game supersedes a half-finished by-hand entry.
  if (picked.length > 0) manual = null;
  onSelectionChanged();
}

async function onSelectionChanged() {
  // Draw first, fetch after. Everything below this point is artwork, which
  // arrives over a network or off a disk; making the whole window wait on it
  // meant ticking a game appeared to do nothing until the cover turned up.
  if (!isCollection()) el.collectionTitle.value = "";
  renderGames();
  renderTickCount();
  refreshRail();

  // A single game brings its own art; a collection has none of its own until it
  // is given some, and each game on it still needs its own poster for the rail.
  if (picked.length === 1 && !art.cover) {
    const game = picked[0];
    try {
      const cover = await invoke("game_cover", { library: game.library, id: game.id });
      if (picked.length === 1 && picked[0].id === game.id) game.cover = cover;
    } catch {
      // no art is a state, not a failure
    }
  }

  if (isCollection()) {
    if (!el.collectionTitle.value.trim()) suggestName();
    // Not awaited: the play order is usable while the posters arrive, and it
    // redraws itself when they do.
    fillGameCovers();
  }

  refreshRail();
}

function renderTickCount() {
  if (manual) {
    el.tickCount.textContent = "by hand";
    return;
  }
  if (picked.length === 0) {
    el.tickCount.textContent = "none ticked";
    return;
  }
  const size = totalBytes();
  el.tickCount.textContent = size
    ? `${picked.length} ticked · ${formatBytes(size)}`
    : `${picked.length} ticked`;
}

async function suggestName() {
  try {
    const name = await invoke("suggest_collection_name", {
      titles: picked.map((g) => g.name),
    });
    el.collectionTitle.placeholder = name || "Game Collection";
  } catch {
    el.collectionTitle.placeholder = `${picked[0]?.name ?? "Games"} and ${picked.length - 1} more`;
  }
  refreshRail();
}

/**
 * Each game's poster, for the play order and the preview.
 *
 * Fetched one at a time and drawn as each arrives. A game whose poster was
 * chosen by hand keeps it.
 */
async function fillGameCovers() {
  for (const game of [...picked]) {
    if (game.cover || game.coverFetched) continue;
    game.coverFetched = true;
    try {
      game.cover = await invoke("game_cover", { library: game.library, id: game.id });
    } catch {
      game.cover = "";
    }
    if (picked.includes(game)) {
      renderOrder();
      refreshRail();
    }
  }
}

/* ==========================================================================
   Step 2 — name it and give it a face
   ========================================================================== */

function collectionName() {
  return el.collectionTitle.value.trim() || el.collectionTitle.placeholder || "Game Collection";
}

/**
 * Squeeze a title into a volume label the chosen filesystem will take.
 *
 * exFAT's 11 characters is the tight one; the limit comes from core rather than
 * from a number written here, which is how the old copy ended up saying 64 for
 * btrfs against a real limit of 256.
 */
function driveLabelFor(title, filesystem = DEFAULT_FILESYSTEM) {
  const limit = filesystemInfo(filesystem)?.labelLimit ?? 11;
  const cleaned = String(title).toUpperCase().replace(/[^A-Z0-9]+/g, "");
  return cleaned.slice(0, limit) || "GAMEPAK";
}

/** An inline SVG, since these are the only pictures in the interface. */
function icon(paths, size = 14) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 16 16");
  svg.setAttribute("width", String(size));
  svg.setAttribute("height", String(size));
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "1.6");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  svg.setAttribute("aria-hidden", "true");
  for (const d of paths) {
    const node = document.createElementNS("http://www.w3.org/2000/svg", "path");
    node.setAttribute("d", d);
    svg.append(node);
  }
  return svg;
}

const ICON_IMAGE = ["M2.4 3.6h11.2v8.8H2.4z", "M2.4 10l3.2-2.6 2.6 2 2-1.4 3.4 2.4", "M5.9 6.3v.01"];
const ICON_CROSS = ["M4.4 4.4l7.2 7.2", "M11.6 4.4l-7.2 7.2"];
const ICON_GRIP = ["M6 4.5h.01", "M10 4.5h.01", "M6 8h.01", "M10 8h.01", "M6 11.5h.01", "M10 11.5h.01"];
const ICON_STAR = ["M8 2.2l1.8 3.7 4 .6-2.9 2.8.7 4L8 11.4l-3.6 1.9.7-4-2.9-2.8 4-.6z"];

/**
 * The game a collection's drive icon is made from: the one starred, else the
 * first. An icon chosen for the cartridge itself still wins over either.
 */
function primaryOf(rows) {
  return rows.find((game) => game.primary) ?? rows[0] ?? null;
}

function primaryIndexOf(rows) {
  return rows.length > 1 ? rows.indexOf(primaryOf(rows)) : null;
}

/**
 * What Explorer will show, by the rule the backend applies: the cartridge's
 * own icon, else (for a collection) the primary game's icon or cover, else
 * `fallback`.
 */
function driveIconPreview(rows, fallback) {
  if (art.icon?.preview) return art.icon.preview;
  const primary = rows.length > 1 ? primaryOf(rows) : null;
  return primary?.icon || primary?.cover || fallback;
}

/**
 * One list of games, drawn the same way wherever it appears.
 *
 * Create orders the games about to be written; Edit orders the ones already on
 * a cartridge. They were two renderers with two different sets of controls —
 * arrows in one, drag in the other — for what is visibly the same list, so a
 * person who learned to drag in Create found arrows in Edit.
 *
 * `rows` is the array being reordered in place. Everything is a picture: a grip
 * to say the row moves, the poster it will use, and a cross to take it off.
 */
function renderGameRows(list, rows, { showSize = false, removable = false, fallbackCover = "", onChange }) {
  list.replaceChildren();
  rows.forEach((game, index) => {
    const li = document.createElement("li");
    li.className = "order-row";
    li.draggable = true;
    li.dataset.index = String(index);

    const grip = document.createElement("span");
    grip.className = "order-row__grip";
    grip.append(icon(ICON_GRIP, 13));

    const n = document.createElement("span");
    n.className = "order-row__n";
    n.textContent = String(index + 1);

    // The poster this game gets on the launcher. Steam and Playnite already
    // have one for most games, so the row shows what will be used rather than
    // an empty slot waiting to be filled in.
    const thumb = document.createElement("span");
    thumb.className = "order-row__art";
    // A single game's art is the cartridge's, so its row borrows that.
    const cover = safeSrc(game.cover) || safeSrc(fallbackCover);
    if (cover) {
      const img = document.createElement("img");
      img.alt = "";
      img.src = cover;
      thumb.append(img);
    } else {
      thumb.classList.add("is-empty");
    }

    const name = document.createElement("span");
    name.className = "order-row__name";
    name.textContent = game.name ?? game.title ?? "";

    li.append(grip, n, thumb, name);

    if (showSize) {
      const size = document.createElement("span");
      size.className = "order-row__size";
      size.textContent = game.sizeOnDisk ? formatBytes(game.sizeOnDisk) : "—";
      li.append(size);
    }

    const poster = document.createElement("button");
    poster.className = "row-btn";
    poster.type = "button";
    // Not draggable, or starting the drag from the button would reorder rather
    // than press.
    poster.draggable = false;
    poster.append(icon(ICON_IMAGE));
    poster.classList.toggle("is-set", Boolean(game.coverSource));
    poster.title = `Poster for ${game.name ?? game.title}`;
    poster.setAttribute("aria-label", poster.title);
    poster.addEventListener("click", (event) => {
      event.stopPropagation();
      openArtwork("cover", Number(li.dataset.index));
    });

    // Which game the drive shows in Explorer. Only a collection has a choice.
    if (rows.length > 1) {
      const star = document.createElement("button");
      star.className = "row-btn";
      star.type = "button";
      star.draggable = false;
      star.append(icon(ICON_STAR));
      const primary = primaryOf(rows) === game;
      star.classList.toggle("is-set", primary);
      star.setAttribute("aria-pressed", String(primary));
      star.title = primary
        ? `${game.name ?? game.title} is the drive icon`
        : `Use ${game.name ?? game.title} as the drive icon`;
      star.setAttribute("aria-label", star.title);
      star.addEventListener("click", (event) => {
        event.stopPropagation();
        for (const other of rows) other.primary = other === game;
        // Picking a game is picking the icon, so a custom one stops winning.
        art.icon = null;
        onChange();
      });
      li.append(star);
    }
    li.append(poster);

    if (removable) {
      const drop = document.createElement("button");
      drop.className = "row-btn row-btn--danger";
      drop.type = "button";
      drop.draggable = false;
      drop.append(icon(ICON_CROSS));
      drop.title = `Take ${game.name ?? game.title} off the cartridge`;
      drop.setAttribute("aria-label", drop.title);
      drop.addEventListener("click", (event) => {
        event.stopPropagation();
        rows.splice(index, 1);
        onChange();
      });
      li.append(drop);
    }

    list.append(li);
  });
}

function renderOrder() {
  renderGameRows(el.orderList, picked, {
    showSize: true,
    onChange: () => {
      renderOrder();
      refreshRail();
    },
  });
}

/* Drag to reorder — the launcher plays them in this order. */
let dragFrom = null;

/**
 * Reordering, for any list drawn by renderGameRows.
 *
 * Attached once per list rather than per row, so a redraw during a drag cannot
 * strand a listener on a row that no longer exists.
 */
function draggable(list, rowsOf, onChange) {
  list.addEventListener("dragstart", (event) => {
    const row = event.target.closest(".order-row");
    if (!row) return;
    dragFrom = Number(row.dataset.index);
    row.classList.add("is-dragging");
    event.dataTransfer.effectAllowed = "move";
    // WebKitGTK refuses to start a drag whose dataTransfer carries nothing, so
    // the index goes in even though the drop reads it from `dragFrom`.
    event.dataTransfer.setData("text/plain", String(dragFrom));
  });

  list.addEventListener("dragover", (event) => {
    event.preventDefault();
    const row = event.target.closest(".order-row");
    for (const other of list.children) other.classList.remove("is-over");
    if (row) row.classList.add("is-over");
  });

  list.addEventListener("drop", (event) => {
    event.preventDefault();
    const row = event.target.closest(".order-row");
    if (row && dragFrom !== null) {
      const to = Number(row.dataset.index);
      // Looked up now, not captured: `picked` is reassigned by ticking and
      // `editing.games` is replaced whenever another cartridge is opened, so a
      // reference taken when the listener was attached goes stale.
      const rows = rowsOf();
      const [moved] = rows.splice(dragFrom, 1);
      rows.splice(to, 0, moved);
      onChange();
    }
    dragFrom = null;
  });

  list.addEventListener("dragend", () => {
    for (const other of list.children) other.classList.remove("is-over", "is-dragging");
    dragFrom = null;
  });
}

draggable(el.orderList, () => picked, () => {
  renderOrder();
  refreshRail();
});

el.collectionTitle.addEventListener("input", () => {
  refreshRail();
  refreshCreateButton();
});


/* ==========================================================================
   Step 1b — a game nothing scanned
   ========================================================================== */

function enterManual() {
  manual = manual ?? { title: "", executable: "", folder: null, rom: null, choices: [], cover: null };
  picked = [];
  showPhase("custom");
  renderFolder();
  renderRom();
  renderExeChoices();
  renderGames();
  renderTickCount();
  refreshRail();
  el.customTitle.focus();
}

async function pickFolder() {
  try {
    const chosen = await invoke("pick_game_folder");
    if (!chosen) return;
    manual = manual ?? { title: "", executable: "", folder: null, rom: null, choices: [], cover: null };
    manual.folder = chosen;
    manual.rom = null;
    renderRom();
    // The folder's own name is the best guess at the title, tidied.
    if (!el.customTitle.value.trim()) {
      el.customTitle.value = tidyFolderName(chosen.name || "");
    }
    manual.choices = chosen.choices ?? [];
    // The backend's best guess, picked already. It is shown first and can be
    // changed with one click; an uninstaller is never picked for anyone.
    const best = [...manual.choices].sort((a, b) => (b.score ?? 0) - (a.score ?? 0))[0];
    if (best && (best.score ?? 0) > 0 && !el.customExec.value.trim()) {
      manual.executable = best.relative;
    }
    renderFolder();
    renderExeChoices();
    refreshRail();
    refreshCreateButton();
  } catch (error) {
    status(String(error), "error");
  }
}

/**
 * "Split_Fiction v1.2" → "Split Fiction".
 *
 * Version tags go before separators are normalised, or "v1.2" turns into
 * "v1 2" and stops looking like a version at all. A guess either way, which is
 * why it is offered as a filled-in field rather than applied silently.
 */
function tidyFolderName(name) {
  return String(name)
    .replace(/\s*[[(].*?[\])]\s*/g, " ")
    .replace(/[_v]?\d+(\.\d+)+\s*$/i, " ")
    .replace(/\bv\d+(\.\d+)*\b/gi, " ")
    .replace(/[._]+/g, " ")
    .replace(/\s{2,}/g, " ")
    .trim();
}

function renderFolder() {
  const chosen = manual?.folder;
  el.folderChosen.hidden = !chosen;
  el.btnClearFolder.hidden = !chosen;
  el.btnPickFolder.textContent = chosen ? "Change folder…" : "Choose folder…";
  if (chosen) {
    el.folderChosen.textContent = chosen.sizeBytes
      ? `${chosen.path} · ${formatBytes(chosen.sizeBytes)}`
      : chosen.path;
  }
}

/**
 * An emulated game: one ROM file, copied into its system's folder on the
 * cartridge. Its extension usually says which system, so the platform is set
 * for the person when it is still PC; an .iso or .zip says nothing and the
 * build waits for them to choose.
 */
async function pickRom() {
  try {
    const chosen = await invoke("pick_rom_file");
    if (!chosen) return;
    manual = manual ?? { title: "", executable: "", folder: null, rom: null, choices: [], cover: null };
    manual.rom = chosen;
    manual.folder = null;
    manual.choices = [];
    manual.executable = "";
    if (!el.customTitle.value.trim()) {
      el.customTitle.value = tidyFolderName(chosen.name || "");
    }
    if (chosen.platform && el.optPlatform.value === "PC") {
      el.optPlatform.value = chosen.platform;
    }
    renderRom();
    renderFolder();
    renderExeChoices();
    refreshRail();
    refreshCreateButton();
  } catch (error) {
    status(String(error), "error");
  }
}

function renderRom() {
  const chosen = manual?.rom;
  el.romChosen.hidden = !chosen;
  el.btnClearRom.hidden = !chosen;
  el.btnPickRom.textContent = chosen ? "Change ROM…" : "Choose ROM…";
  // A ROM is what Play opens, so a link would only be ignored.
  el.customExec.closest(".field").hidden = Boolean(chosen);
  el.exeChoices.closest(".field").hidden = Boolean(chosen);
  if (chosen) {
    el.romChosen.textContent = `${chosen.path} · ${formatBytes(chosen.sizeBytes)}`;
  }
}

/**
 * The executables in the folder, best guess first.
 *
 * The backend scores them; helper runtimes, crash handlers and uninstallers
 * come back negative and are shown last, saying why rather than being hidden —
 * sometimes the odd-looking one really is the game.
 */
function renderExeChoices() {
  const choices = [...(manual?.choices ?? [])].sort((a, b) => (b.score ?? 0) - (a.score ?? 0));
  el.exeChoices.replaceChildren();

  if (choices.length === 0) {
    el.exeHint.textContent = manual?.folder
      ? "No programs found. Type a link below."
      : "";
    return;
  }

  for (const choice of choices) {
    const li = document.createElement("li");
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "exe";
    btn.classList.toggle("is-demoted", (choice.score ?? 0) < 0);
    btn.setAttribute("aria-pressed", String(manual?.executable === choice.relative));

    const name = document.createElement("span");
    name.className = "exe__name";
    name.textContent = choice.relative;
    btn.append(name);

    const why = whyDemoted(choice);
    if (why) {
      const tag = document.createElement("span");
      tag.className = "exe__why";
      tag.textContent = why;
      btn.append(tag);
    }

    btn.addEventListener("click", () => {
      manual.executable = choice.relative;
      el.customExec.value = "";
      renderExeChoices();
      refreshCreateButton();
    });

    li.append(btn);
    el.exeChoices.append(li);
  }

  el.exeHint.textContent = "Pick the one that starts the game.";
}

function whyDemoted(choice) {
  const name = String(choice.relative ?? "").toLowerCase();
  if (/unins|uninstall/.test(name)) return "uninstaller";
  if (/(^|\/)(ue4prereqsetup|vcredist|dxsetup|directx|dotnet)/.test(name)) return "runtime";
  if (/crashp|crashre|handler/.test(name)) return "crash handler";
  if ((choice.score ?? 0) < 0) return "probably not the game";
  return "";
}

/* ==========================================================================
   Drives
   ========================================================================== */

/**
 * The drive list, for whichever picker asked.
 *
 * Create offers every writable drive; Edit offers only the ones that already
 * hold a cartridge, because editing one that does not exist is not a thing.
 */
function renderDrives({ onlyCartridges = false, choose = selectDrive } = {}) {
  el.drives.replaceChildren();
  // Every drive, every time. This once collapsed to the chosen one, which made
  // sense while the list sat in the rail and made none at all once the list
  // became the picker: opening "Change" to change the drive showed one drive,
  // the one you were trying to change away from.
  const shown = onlyCartridges ? drives.filter((d) => d.hasCartridge) : drives;
  for (const drive of shown) {
    const li = document.createElement("li");
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "drive";
    const current = onlyCartridges ? editing?.drivePath : selectedDrive;
    btn.setAttribute("aria-selected", String(drive.path === current));

    const name = document.createElement("span");
    name.className = "drive__name";
    // Editing is about what is on the drive, so it leads with that; the volume
    // label only changes when a drive is erased and is often an older game's.
    name.textContent = onlyCartridges
      ? drive.cartridgeTitle || drive.label || drive.path
      : drive.label || drive.path;

    const meta = document.createElement("span");
    meta.className = "drive__meta";
    meta.textContent = onlyCartridges
      ? `${drive.label} · ${formatBytes(drive.freeBytes)} free`
      : drive.hasCartridge
        ? `${formatBytes(drive.freeBytes)} free · has ${drive.cartridgeTitle || "a cartridge"}`
        : `${formatBytes(drive.freeBytes)} free of ${formatBytes(drive.totalBytes)}`;
    if (drive.hasCartridge && !onlyCartridges) meta.classList.add("drive__warn");

    btn.append(name, meta);
    btn.addEventListener("click", () => choose(drive));
    li.append(btn);
    el.drives.append(li);
  }

  // A cartridge on a volume Windows never lettered is invisible to every
  // other part of this: it has no path to write to and no path to read from,
  // so it cannot be listed above and could not be found by looking harder.
  // Offering it here, as a drive that needs a letter, is the only way anyone
  // finds out it is there at all — short of opening Disk Management.
  for (const volume of unmounted) {
    const li = document.createElement("li");
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "drive drive--unmounted";

    const name = document.createElement("span");
    name.className = "drive__name";
    // A volume Windows never mounted has no label to read either, so there is
    // often nothing here but the size and the disk it sits on.
    name.textContent =
      volume.label ||
      (volume.filesystem
        ? `${volume.filesystem} volume`
        : `Unnamed volume on disk ${volume.disk}`);

    const meta = document.createElement("span");
    meta.className = "drive__meta drive__warn";
    meta.textContent = `${formatBytes(volume.totalBytes)} · needs a drive letter`;

    btn.append(name, meta);
    btn.addEventListener("click", () => mountVolume(volume, btn));
    li.append(btn);
    el.drives.append(li);
  }

  el.drivesEmpty.hidden = shown.length > 0 || unmounted.length > 0;
  if (shown.length === 0) {
    el.drivesEmpty.textContent = onlyCartridges
      ? "No cartridge plugged in."
      : "No removable drive found. Plug one in.";
  }
}

/**
 * Ask Windows to give this volume a drive letter, then pick it up as a drive.
 *
 * Elevates on the backend, so there is a UAC prompt in the middle of it and a
 * decline comes back as an error rather than a silent nothing.
 */
async function mountVolume(volume, button) {
  button.disabled = true;
  status(`Assigning a drive letter to ${volume.label || "the volume"}…`);
  let path;
  try {
    path = await invoke("mount_volume", { volume });
  } catch (error) {
    button.disabled = false;
    status(String(error), "error");
    return;
  }
  await refreshDrives();
  const drive = drives.find((d) => d.path === path);
  if (drive) {
    status(`${drive.label} is ready.`);
    await selectDrive(drive);
  } else {
    status(`Mounted at ${path}.`);
  }
}

/* ==========================================================================
   Change game / Change media

   The library and the drive list did not move into a new widget — they moved
   into a dialog. Same markup, same renderers, same ticking, so multi-select,
   search, "Enter it by hand" and the drag order all behave exactly as they did
   when this list was step 1.
   ========================================================================== */

function openPick(kind) {
  const games = kind === "game";
  el.pickTitle.textContent = games ? "Choose games" : "Choose a drive";
  if (!games) renderDrives();
  el.pickGames.hidden = !games;
  el.pickMedia.hidden = games;
  el.pickDialog.showModal();
  if (games) el.search.focus();
}

function openGamePicker() {
  openPick("game");
}

function openMediaPicker() {
  openPick("media");
  // Open on what is known, then fill in what has been plugged in since.
  if (!building) void refreshDrives();
}

async function selectDrive(drive) {
  selectedDrive = drive.path;
  renderDrives();
  // One drive is the whole answer, so the dialog has nothing left to ask.
  if (el.pickDialog.open && !el.pickMedia.hidden) el.pickDialog.close();

  formatPlan = null;
  try {
    formatPlan = await invoke("format_plan", { drivePath: drive.path });
  } catch {
    formatPlan = null;
  }

  try {
    driveIsSteamLibrary = Boolean(
      await invoke("steam_registration", { drivePath: drive.path }),
    );
  } catch {
    driveIsSteamLibrary = false;
  }

  // A cartridge can hold a Steam game and not be a Steam library: Steam drops a
  // library whose drive was missing when it started, which for a cartridge is
  // most of the time. The game is still there; Steam has just stopped believing
  // in it, and will re-download rather than use it.
  //
  // Being listed is not enough either. Steam records a game as installed in one
  // library, so the one it was copied from has to stop claiming it — which is
  // why the plan can have something to say about a cartridge that is already
  // registered.
  try {
    driveHoldsSteamGames = Boolean(
      await invoke("holds_steam_games", { drivePath: drive.path }),
    );
  } catch {
    driveHoldsSteamGames = false;
  }

  try {
    steamPlan = driveHoldsSteamGames
      ? await invoke("steam_registration_plan", { drivePath: drive.path })
      : [];
  } catch {
    steamPlan = [];
  }
  steamConfirmed = false;
  el.btnRegister.textContent = "Fix this cartridge in Steam";

  await readLinkRate(drive.path);

  el.btnUnregister.hidden = !driveIsSteamLibrary;
  el.btnRegister.hidden = steamPlan.length === 0;
  refreshRail();
}

/* ==========================================================================
   What gets written
   ========================================================================== */

/**
 * Every filesystem a cartridge can be made with, as core describes them.
 *
 * Fetched rather than written out here: the wizard used to carry its own list
 * with the label limit repeated, and the copy had already drifted from the real
 * one. Empty until `loadFilesystems` has run, so every reader falls back.
 */
let filesystems = [];

const DEFAULT_FILESYSTEM = "ntfs";

function filesystemInfo(id) {
  return filesystems.find((f) => f.id === id) ?? null;
}

/** The filesystem this cartridge is erased to. Starts from Settings. */
function filesystem() {
  const wanted = run.filesystem;
  // An id from a newer build, or a hand-edited settings file, must not become
  // a format nobody can make.
  return filesystemInfo(wanted) ? wanted : DEFAULT_FILESYSTEM;
}

function filesystemLabel(fs) {
  return filesystemInfo(fs)?.name ?? fs ?? "NTFS";
}

/** Fill the Settings picker, and say what each choice costs. */
async function loadFilesystems() {
  try {
    filesystems = await invoke("list_filesystems");
  } catch (error) {
    console.error("could not list filesystems", error);
    return;
  }

  // Two pickers, one list: the default in Settings and this cartridge's own.
  for (const select of [el.setFilesystem, el.optFilesystem]) {
    select.replaceChildren();
    // Only what this machine can actually make, by name. The note under the
    // picker says what each one costs.
    for (const fs of filesystems.filter((fs) => fs.canCreateHere)) {
      const option = document.createElement("option");
      option.value = fs.id;
      option.textContent = fs.name;
      select.append(option);
    }
  }
  el.setFilesystem.value = filesystemInfo(settings.defaultFilesystem)
    ? settings.defaultFilesystem
    : DEFAULT_FILESYSTEM;
  describeFilesystem();
}

/** One line: where it works, and whether Proton does. */
function filesystemNote(id) {
  const fs = filesystemInfo(id);
  if (!fs) return "";
  const where = (fs.nativeOn ?? []).join(", ");
  const proton = fs.runsProton ? "Proton works" : "no Proton";
  return where ? `${where} · ${proton}` : proton;
}

function describeFilesystem() {
  if (!el.setFilesystemNote) return;
  el.setFilesystemNote.textContent = filesystemNote(el.setFilesystem.value);
}

/** Every game that would actually have its files copied. */
function copyable() {
  if (manual) return manual.folder || manual.rom ? [manual] : [];
  return picked.filter((g) => g.sizeOnDisk > 0);
}

/* ==========================================================================
   Rate and time
   --------------------------------------------------------------------------
   An estimate has to come from somewhere real or it should not be shown. The
   write rate starts as a conservative figure for a USB-attached NVMe, and is
   replaced by the measured rate the moment the copy is actually running — so
   the number on the button before the write is an estimate, and the number
   during it is an observation.
   ========================================================================== */

/**
 * What to assume before anything has been measured.
 *
 * A cartridge is only as quick as the link it is on, and the launcher already
 * asks the drive what that is. A negotiated link gives a ceiling; the fraction
 * of it a real copy reaches is well under the theoretical figure, because a
 * game is thousands of files rather than one long stream. When the link cannot
 * be read at all, this is the fallback — deliberately pessimistic, so an
 * estimate comes in early rather than late.
 */
const FALLBACK_WRITE_BYTES_PER_S = 120_000_000;

/** Sustained many-file throughput as a share of the negotiated link. */
const LINK_EFFICIENCY = 0.45;

let measuredRate = null;
let linkRate = null;

function writeRate() {
  return measuredRate ?? linkRate ?? FALLBACK_WRITE_BYTES_PER_S;
}

/** Ask the drive what it is connected at, so the estimate has a basis. */
async function readLinkRate(drivePath) {
  linkRate = null;
  try {
    const health = await invoke("cartridge_health", { drivePath });
    if (typeof health?.linkMbps === "number" && health.linkMbps > 0) {
      linkRate = (health.linkMbps * 1_000_000 / 8) * LINK_EFFICIENCY;
    }
  } catch {
    // an estimate without it is still an estimate
  }
}

/** Reading back to verify is faster than writing, but not free. */
function readRate() {
  return writeRate() * 1.6;
}

/** Seconds the whole build is expected to take, from what is ticked. */
function estimateSeconds() {
  let seconds = 0;
  if (on.format) seconds += 20;
  if (on.closeSteam) seconds += 5;
  const size = on.copy ? copyable().reduce((s, g) => s + (g.sizeOnDisk || (g.folder ?? g.rom)?.sizeBytes || 0), 0) : 0;
  if (size) seconds += size / writeRate();
  if (on.verify && size) seconds += size / readRate();
  seconds += 2; // conf, launcher, manifest
  return seconds;
}

/* ==========================================================================
   The plan: what is ticked, in the order it will happen
   ========================================================================== */

function planSteps() {
  const steps = [];
  const size = on.copy
    ? copyable().reduce((s, g) => s + (g.sizeOnDisk || (g.folder ?? g.rom)?.sizeBytes || 0), 0)
    : 0;

  if (on.format && formatPlan) {
    steps.push({
      what: `Format ${formatPlan.currentLabel} as ${filesystemLabel(filesystem())}`,
      detail: `renamed ${driveLabelFor(cartridgeTitle(), filesystem())} · ~20 s`,
      danger: true,
    });
  }
  if (on.copy && on.closeSteam) {
    steps.push({ what: "Close Steam" });
  }
  if (size) {
    const folders = copyable().length;
    steps.push({
      what: `Copy ${formatBytes(size)}`,
      detail: `${folders} folder${folders === 1 ? "" : "s"} · ${formatDuration(size / writeRate())}`,
    });
  }
  if (on.verify && size) {
    steps.push({ what: "Verify the copy", detail: formatDuration(size / readRate()) });
  }
  if (settings.hltbEnabled) {
    steps.push({ what: "Look up how long to beat", detail: "HowLongToBeat" });
  }
  steps.push({
    what: on.icon
      ? "Write launcher, icon and manifest"
      : "Write the launcher and manifest",
    detail: on.icon ? "autorun.ico · ~2 s" : "~2 s",
  });
  // After the write, because that is when the backend does it: the read-back
  // that makes deleting safe is the last thing the build does before this.
  if (on.move) {
    const games = movable();
    const freed = games.reduce((s, g) => s + (g.sizeOnDisk || 0), 0);
    steps.push({
      what:
        games.length === 1
          ? `Delete ${games[0].name} from this PC`
          : `Delete ${games.length} games from this PC`,
      detail: `only if every file verified · frees ${formatBytes(freed)}`,
      danger: true,
    });
  }
  if (on.tune) {
    steps.push({ what: "Tune Windows for this cartridge", detail: "Defender and Search" });
  }
  if (on.trim) {
    steps.push({ what: "Release freed space", detail: "needs the format permission" });
  }
  if (on.eject) {
    // Only a formatted drive answers to the new name; otherwise it keeps its own.
    const name = on.format
      ? driveLabelFor(cartridgeTitle(), filesystem()) || driveLabel()
      : driveLabel();
    steps.push({ what: `Eject ${name}` });
  }
  return steps;
}

function driveLabel() {
  return drives.find((d) => d.path === selectedDrive)?.label ?? "the cartridge";
}

function refreshPlan() {
  // The plan is only worth showing once there is a drive for it to happen to.
  const show = !finished && selectedDrive && (picked.length > 0 || manual);
  el.railPlan.hidden = !show;
  if (!show) return;

  el.plan.replaceChildren();
  planSteps().forEach((step, index) => {
    const li = document.createElement("li");
    li.className = "plan-row";
    if (step.danger) li.classList.add("plan-row--danger");

    const n = document.createElement("span");
    n.className = "plan-row__n";
    n.textContent = String(index + 1);

    const body = document.createElement("span");
    const what = document.createElement("span");
    what.className = "plan-row__what";
    what.textContent = step.what;
    body.append(what);
    if (step.detail) {
      const detail = document.createElement("span");
      detail.className = "plan-row__detail";
      detail.textContent = step.detail;
      body.append(detail);
    }

    li.append(n, body);
    el.plan.append(li);
  });
}

/* ==========================================================================
   The rail
   ========================================================================== */

function cartridgeTitle() {
  if (manual) return el.customTitle.value.trim() || "Untitled";
  if (isCollection()) return collectionName();
  return picked[0]?.name ?? "";
}

function refreshRail() {
  if (activeTab === "edit") {
    el.barText.textContent = editing?.title ? `Edit ${editing.title}` : "Edit cartridge";
    el.railEmpty.hidden = Boolean(editing);
    el.railBody.hidden = !editing;
    if (editing) {
      const games = editing.games ?? [];
      el.railKindText.textContent = games.length > 1 ? "Multicartridge" : "Cartridge";
      el.railKindCount.hidden = games.length <= 1;
      el.railKindCount.textContent = `${games.length} games`;
      // A collection opens on its first game, with that game's art and logo.
      const collection = games.length > 1;
      renderRailLauncher(collection ? games[0]?.cover || editing.cover : editing.cover, {
        title: el.editTitle.value.trim() || editing.title,
        first: games[0]?.title,
        games: games.length,
        logo: collection
          ? games[0]?.logo
          : art.logo?.remove ? null : art.logo?.preview ?? editing.logo,
      });
      el.railSpace.hidden = true;
      el.railPlan.hidden = true;
    }
    refreshCreateButton();
    return;
  }

  const has = picked.length > 0 || Boolean(manual);
  el.railEmpty.hidden = has;
  el.railBody.hidden = !has;

  // The build screen is the left column, not part of the rail, so it renders
  // whether or not a game has been chosen — choosing the media before the game
  // is a perfectly ordinary way round, and the card has to say so.
  renderBuild();

  if (!has) {
    refreshCreateButton();
    return;
  }

  // Cartridge or Multicartridge — the panel titles itself to the tick count.
  const collection = isCollection();
  el.railKindText.textContent = collection ? "Multicartridge" : "Cartridge";
  el.railKindCount.hidden = !collection;
  el.railKindCount.textContent = `${picked.length} games`;
  el.barText.textContent = building
    ? `Writing ${on.format ? driveLabelFor(cartridgeTitle(), filesystem()) || driveLabel() : driveLabel()}`
    : collection
      ? "Create multicartridge"
      : "Create cartridge";

  // The rail shows what the cartridge will look like. What it *is* — the game,
  // the drive, the artwork — is stated by the cards in the left column, so
  // repeating any of it here would be two places to keep in step.
  // A collection's launcher opens on its first game, showing that game's art.
  const coverSrc = collection
    ? picked[0]?.cover
    : art.cover?.preview ?? picked[0]?.cover;
  renderRailLauncher(coverSrc);

  renderBuild();
  refreshSpace();
  refreshMove();
  refreshPlan();
  refreshCreateButton();
}

/**
 * Offer the move only where it means something: Steam games being copied.
 *
 * A Steam game still installed on this PC is the copy Steam plays, however good
 * the one on the cartridge is — Steam lists an app in one library and ignores
 * the rest. The hint says so, because that is the only reason to tick this.
 */
function refreshMove() {
  const games = movable();
  const offered = on.copy && games.length > 0;
  el.moveRow.hidden = !offered;
  if (!offered) return;

  el.optMove.disabled = !on.verify;
  el.optMove.checked = on.move;
  if (!on.verify) {
    el.moveHint.textContent =
      "Needs “Verify after copying” in Settings: nothing is deleted on the strength of an unchecked copy.";
    return;
  }
  const what = games.length === 1 ? games[0].name : `these ${games.length} Steam games`;
  el.moveHint.textContent =
    `Deletes ${what} from this PC once every file on the cartridge has been checked, ` +
    "and tells Steam the cartridge is where it lives. Without this, Steam keeps " +
    "playing the copy on this PC.";
}

/**
 * Keep the preview scaled to whatever width the rail gives it.
 *
 * The card is laid out at the launcher's real 420x560 so every size inside it is
 * the size the launcher itself uses. Fitting it to the rail is therefore one
 * number, and CSS cannot work it out: `scale()` needs a unitless factor and
 * there is no way to divide a container width by 420 to get one.
 */
function fitRailLauncher() {
  const frame = el.railLauncherCard?.parentElement;
  if (!frame) return;
  const width = frame.clientWidth;
  if (width > 0) el.railLauncherCard.style.setProperty("--rail-scale", String(width / 420));
}

if (el.railLauncherCard?.parentElement && "ResizeObserver" in window) {
  new ResizeObserver(fitRailLauncher).observe(el.railLauncherCard.parentElement);
}

/**
 * The launcher, as this cartridge will look when it is plugged in.
 *
 * Built from the same three parts the real one is — the cover behind, the logo
 * or the title over it, Play in the accent — because the question the wizard
 * cannot otherwise answer is whether the artwork and the title survive being put
 * on top of each other. A cover thumbnail on its own never shows that.
 *
 * `--rail-accent` is set from the cover the same way the launcher samples it, so
 * a dark cover gets a light Play and a bright one gets dark ink, here as there.
 */
function renderRailLauncher(coverSrc, override = null) {
  const collection = override ? (override.games ?? 0) > 1 : isCollection();
  setPlate(el.railLauncherCard, el.railLauncherArt, coverSrc);

  // Edit hands in what the cartridge already says, so the preview answers the
  // same question there as here: what will this look like when it is plugged
  // in — which is most of what editing a cartridge is for.
  if (override) {
    showRailLogo(override.logo);
    el.railLauncherEyebrow.hidden = !collection;
    el.railLauncherEyebrow.textContent = collection ? override.title || "Collection" : "";
    const name = (collection ? override.first : override.title) || override.title || "Nothing yet";
    el.railLauncherTitle.textContent = name;
    el.railLauncherTitle.classList.toggle("is-long", name.length > 22);
    el.railExplorer.hidden = true;
    return;
  }

  // A collection prints the selected game's own logo, as the launcher does,
  // with the collection's name above it.
  showRailLogo(collection ? picked[0]?.logo : art.logo?.preview);
  el.railLauncherEyebrow.hidden = !collection;
  el.railLauncherEyebrow.textContent = collection ? cartridgeTitle() || "Collection" : "";

  // With a logo the title is already printed in the artwork; without one this
  // is the launcher's heading, so it shows the game rather than the collection.
  const heading = collection
    ? picked[0]?.name || cartridgeTitle() || "Nothing yet"
    : cartridgeTitle() || "Nothing yet";
  el.railLauncherTitle.textContent = heading;
  el.railLauncherTitle.classList.toggle("is-long", heading.length > 22);

  renderRailExplorer();
}

/** Name the logo slot for what it does on this kind of cartridge. */
function labelLogoSlot(plate, collection) {
  const slot = plate.closest(".art-slot");
  slot.querySelector(".art-slot__name").textContent = collection ? "Collection logo" : "Logo";
  slot.title = collection
    ? "Shown in the launcher's corner. Each game's own logo is set from its row."
    : "";
}

function showRailLogo(src) {
  const logoSrc = safeSrc(src);
  el.railLauncherLogo.hidden = !logoSrc;
  if (logoSrc) el.railLauncherLogo.src = logoSrc;
  el.railLauncherStage.classList.toggle("has-logo", Boolean(logoSrc));
}

/** The drive as Explorer will label it — the one thing the icon slot is for. */
function renderRailExplorer() {
  el.railExplorerLabel.textContent = platform === "windows" ? "In Explorer" : "In the file manager";
  const drive = drives.find((d) => d.path === selectedDrive);
  const named = on.icon;
  el.railExplorer.hidden = !drive || !named;
  if (!drive) return;

  const icon = safeSrc(driveIconPreview(picked, art.cover?.preview ?? picked[0]?.cover));
  el.railExplorerIcon.style.backgroundImage = icon ? `url("${icon}")` : "";
  el.railExplorerIcon.classList.toggle("is-blank", !icon);

  const label = on.format ? driveLabelFor(cartridgeTitle(), filesystem()) : driveLabel();
  const name = label || cartridgeTitle() || "Cartridge";
  // The path is only worth appending when it says something the name does not.
  // It usually does not: on Windows `drive.label` is already "STARDEW (G:)", and
  // on Linux the mount point is named after the label, so a naive append gives
  // "STARDEW (G:) (G:)" or "CINDER (CINDER)".
  const where = shortPath(drive.path);
  const said =
    name.toLowerCase() === where.toLowerCase() ||
    name.toLowerCase().endsWith(`(${where.toLowerCase()})`);
  el.railExplorerName.textContent = said ? name : `${name} (${where})`;
  el.railExplorerFree.textContent = `${formatBytes(drive.freeBytes)} free`;
}

/**
 * How a drive is worth naming in a sentence: `E:` on Windows, and the mount
 * point's own name elsewhere.
 *
 * Explorer's row has one line to play with, so printing
 * `/run/media/harry/CINDER` in it pushes out the thing the row is actually for.
 */
function shortPath(path) {
  const trimmed = String(path ?? "").replace(/[\\/]+$/, "");
  if (/^[A-Za-z]:$/.test(trimmed)) return trimmed;
  const last = trimmed.split(/[\\/]/).filter(Boolean).pop();
  return last || trimmed || "?";
}

/**
 * The build screen: what is chosen, on one page.
 *
 * Every card here is a statement of the current answer with the way to change
 * it beside it. Nothing is a step, so nothing has to be finished before the
 * next thing can be looked at — which is the point of the rebuild.
 */
function renderBuild() {
  const collection = isCollection();
  const first = picked[0];
  el.phases.build.classList.toggle("has-game", picked.length > 0 || Boolean(manual));

  // --- Game -------------------------------------------------------------
  const coverSrc = art.cover?.preview ?? (collection ? null : first?.cover);
  setPlate(el.buildGamePlate, el.buildGameCover, coverSrc);

  if (manual) {
    el.buildGameTitle.textContent = el.customTitle.value.trim() || manual.title || "Entered by hand";
    el.buildGameMeta.textContent = manual.rom
      ? `ROM · ${el.optPlatform.selectedOptions[0]?.textContent ?? ""} · ${formatBytes(manual.rom.sizeBytes)}`
      : manual.folder
        ? `By hand · ${formatBytes(manual.folder.sizeBytes)}`
        : "By hand";
  } else if (collection) {
    el.buildGameTitle.textContent = cartridgeTitle() || `${picked.length} games`;
    el.buildGameMeta.textContent = `${picked.length} games · ${formatBytes(totalBytes())}`;
  } else if (first) {
    el.buildGameTitle.textContent = first.name;
    const bits = [];
    if (first.source) bits.push(first.source);
    bits.push(first.sizeOnDisk ? formatBytes(first.sizeOnDisk) : "not installed");
    el.buildGameMeta.textContent = bits.join(" · ");
  } else {
    el.buildGameTitle.textContent = "No game chosen";
    el.buildGameMeta.textContent = "";
  }
  el.changeGame.textContent = picked.length || manual ? "Change" : "Choose";

  // Naming and ordering only mean anything with more than one game on it.
  el.buildName.hidden = !collection;
  if (collection) {
    renderOrder();
  }

  // --- Media ------------------------------------------------------------
  const drive = drives.find((d) => d.path === selectedDrive);
  if (drive) {
    el.buildMediaTitle.textContent = drive.label || drive.path;
    const bits = [`${formatBytes(drive.freeBytes)} free of ${formatBytes(drive.totalBytes)}`];
    if (drive.hasCartridge) bits.push(`has ${drive.cartridgeTitle || "a cartridge"}`);
    el.buildMediaMeta.textContent = bits.join(" · ");
  } else {
    el.buildMediaTitle.textContent = "No drive chosen";
    el.buildMediaMeta.textContent = drives.length ? "" : "plug a drive in";
  }
  el.changeMedia.textContent = drive ? "Change" : "Choose";

  renderOptions();


  // --- Artwork ----------------------------------------------------------
  el.coverSlot.hidden = collection;
  // A collection's own logo is the mark in the launcher's corner — a SEGA for
  // a shelf of Mega Drive games — while each game's logo is set from its row.
  labelLogoSlot(el.slotLogo, collection);
  setPlate(el.collectionCover, el.collectionCoverImg, coverSrc);
  setPlate(el.slotLogo, el.slotLogoImg, art.logo?.preview);
  setPlate(el.slotIcon, el.slotIconImg, driveIconPreview(picked, coverSrc));
  // No fallback to the cover: a hero is a different shape, and standing a
  // portrait in for one would look like a hero had been chosen when none had.
  setPlate(el.slotHero, el.slotHeroImg, art.background?.preview);
  el.btnInheritCover.textContent = first ? `Use ${first.name}'s artwork` : "";
  el.btnInheritCover.hidden = !collection || !first || Boolean(art.cover);
}


/**
 * What it costs on the drive — one band per game.
 *
 * A stack of bands rather than one bar, so it is visible which game is taking
 * the room before anything is written.
 */
function refreshSpace() {
  const drive = drives.find((d) => d.path === selectedDrive);
  const list = copyable();
  const size = list.reduce((s, g) => s + (g.sizeOnDisk || (g.folder ?? g.rom)?.sizeBytes || 0), 0);

  el.railSpace.hidden = !drive || size === 0;
  if (el.railSpace.hidden) return;

  const capacity = on.format ? drive.totalBytes : drive.freeBytes;
  el.railDriveLabel.textContent = drive.label || drive.path;
  el.railDriveFill.textContent = `${formatBytes(size)} of ${formatBytes(drive.totalBytes)}`;

  el.railBar.replaceChildren();
  el.railBar.classList.toggle("is-over", size > capacity);
  // The band widths are the grid's track sizes, set in one go on the container,
  // so the transition has a single property to interpolate rather than one per
  // band. The bands themselves carry only their colour.
  const tracks = [];
  list.forEach((game, index) => {
    const bytes = game.sizeOnDisk || (game.folder ?? game.rom)?.sizeBytes || 0;
    tracks.push(`${Math.max(1, (bytes / drive.totalBytes) * 100)}%`);
    const band = document.createElement("div");
    // Each band a step further round the accent's hue, so they read as one
    // family rather than a chart.
    band.style.background = `color-mix(in oklch, var(--accent) ${100 - index * 12}%, oklch(0.5 0.12 20))`;
    el.railBar.append(band);
  });
  el.railBar.style.gridTemplateColumns = tracks.join(" ");
}

/* ==========================================================================
   Options — this cartridge's choices
   ========================================================================== */

function renderOptions() {
  const canCopy = copyable().length > 0;
  el.optCopy.checked = on.copy;
  el.optCopy.disabled = !canCopy;
  el.optCopyRow.classList.toggle("is-off", !canCopy);
  const size = copyable().reduce((s, g) => s + (g.sizeOnDisk || (g.folder ?? g.rom)?.sizeBytes || 0), 0);
  el.optCopyMeta.textContent = !(picked.length || manual)
    ? ""
    : canCopy
      ? formatBytes(size)
      : "not installed";

  // Verifying means reading back a copy, so without one there is nothing to do.
  el.optVerifyRow.hidden = !on.copy;
  el.optVerify.checked = run.verify;

  el.optEject.checked = run.eject;

  el.optFormat.checked = run.format;
  el.optFormat.disabled = !selectedDrive;
  el.optFormatRow.classList.toggle("is-on", on.format);
  el.optFilesystem.hidden = !on.format;
  el.optFilesystem.value = filesystem();
  el.optFilesystem.title = filesystemNote(filesystem());
}

el.optCopy.addEventListener("change", () => {
  run.copy = el.optCopy.checked;
  refreshRail();
});
el.optVerify.addEventListener("change", () => {
  run.verify = el.optVerify.checked;
  refreshRail();
});
el.optEject.addEventListener("change", () => {
  run.eject = el.optEject.checked;
  refreshRail();
});
el.optFormat.addEventListener("change", () => {
  run.format = el.optFormat.checked;
  refreshRail();
});
el.optFilesystem.addEventListener("change", () => {
  run.filesystem = el.optFilesystem.value;
  refreshRail();
});

/* ==========================================================================
   The button, and what it is waiting for
   ========================================================================== */

function blockingReason() {
  if (picked.length === 0 && !manual) return "Choose a game";
  if (manual && !manual.executable && !el.customExec.value.trim() && !manual.folder && !manual.rom) {
    return "Choose what Play starts";
  }
  if (manual?.rom && el.optPlatform.value === "PC") return "Choose the ROM's platform";
  if (manual && !el.customTitle.value.trim()) return "Give it a title";
  if (!selectedDrive) return "Choose a drive";

  const drive = drives.find((d) => d.path === selectedDrive);
  const size = on.copy
    ? copyable().reduce((s, g) => s + (g.sizeOnDisk || (g.folder ?? g.rom)?.sizeBytes || 0), 0)
    : 0;
  const capacity = on.format ? drive?.totalBytes : drive?.freeBytes;
  if (size > 0 && drive && size > capacity) {
    return `Needs ${formatBytes(size - capacity)} more space`;
  }

  if (on.format) {
    // No typed confirmation to wait for — formatting is a setting now. The plan
    // is still required, because it is what states on screen which drive is
    // about to be erased and what is on it.
    if (!formatPlan) return "This drive cannot be erased";
    if (!driveLabelFor(cartridgeTitle(), filesystem())) return "Name the drive";
  }
  return "";
}

function refreshCreateButton() {
  if (building) {
    el.create.disabled = true;
    return;
  }
  if (pendingArt > 0) {
    el.create.disabled = true;
    el.readyMessage.textContent = "Downloading artwork…";
    el.readyMessage.className = "";
    return;
  }

  // Edit writes changes to a cartridge that exists. Same button, same place,
  // one word different — there is no second Save tucked into the panel.
  if (activeTab === "edit") {
    el.create.disabled = !editing;
    el.createLabel.textContent = "Update cartridge";
    el.readyMessage.textContent = editing
      ? `Changing ${editing.title || "this cartridge"}`
      : "Choose a cartridge to edit";
    el.readyMessage.className = "";
    return;
  }

  if (finished) {
    el.create.disabled = false;
    el.createLabel.textContent = "Make another";
    el.readyMessage.textContent = "Done";
    el.readyMessage.className = "is-ready";
    return;
  }

  const collection = isCollection();
  const reason = blockingReason();
  const ready = reason === "";
  el.create.disabled = !ready;

  el.createLabel.textContent = on.format
    ? "Erase and write"
    : collection
      ? "Write multicartridge"
      : "Write cartridge";

  if (!ready) {
    el.readyMessage.textContent = reason;
    el.readyMessage.className = "";
    return;
  }

  const seconds = estimateSeconds();
  const size = on.copy
    ? copyable().reduce((s, g) => s + (g.sizeOnDisk || (g.folder ?? g.rom)?.sizeBytes || 0), 0)
    : 0;

  if (on.format && formatPlan) {
    el.readyMessage.textContent =
      `Erases ${formatPlan.currentLabel} · ${size ? `${formatBytes(size)}, ` : ""}${formatDuration(seconds)}`;
    el.readyMessage.className = "is-destructive";
  } else {
    el.readyMessage.textContent = size
      ? `${formatBytes(size)}, ${formatDuration(seconds)}`
      : formatDuration(seconds);
    el.readyMessage.className = "is-ready";
  }
}

/* ==========================================================================
   Writing
   ========================================================================== */

/** One row per step of the plan, ticked off as the build reports them. */
function renderLog(steps, doneCount) {
  el.log.replaceChildren();
  steps.forEach((step, index) => {
    const li = document.createElement("li");
    li.className = "log-row";
    if (index < doneCount) li.classList.add("is-done");
    else if (index === doneCount) li.classList.add("is-now");

    const mark = document.createElement("span");
    mark.className = "log-row__mark";
    if (index < doneCount) {
      mark.innerHTML =
        '<svg viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" width="8" height="8" aria-hidden="true"><path d="M2.4 6.2l2.2 2.2 5-5.4" /></svg>';
    }

    const text = document.createElement("span");
    text.textContent = step.what;

    li.append(mark, text);
    el.log.append(li);
  });
}

/** The options that are in force, so the running window still says what it is doing. */
function renderForced() {
  const rows = [
    [on.format, `Formatted as ${filesystemLabel(filesystem())}`, "Not formatted"],
    [on.copy, "Games copied onto the cartridge", "Nothing copied — a key only"],
    [on.verify && on.copy, "Verify after copying", "Copy not verified"],
    [on.move, "PC's Steam install deleted once verified", "PC's install kept"],
    [on.icon, "Drive icon from artwork", "No drive icon"],
    [on.tune, "Windows tuned", "Windows tuning skipped"],
    [on.eject, "Eject when done", "Left mounted"],
  ];

  el.forced.replaceChildren();
  for (const [on, yes, no] of rows) {
    const li = document.createElement("li");
    li.className = on ? "forced-row" : "forced-row is-off";
    const mark = document.createElement("span");
    mark.className = "forced-row__mark";
    mark.textContent = on ? "✓" : "—";
    const text = document.createElement("span");
    text.textContent = on ? yes : no;
    li.append(mark, text);
    el.forced.append(li);
  }
}

let runStartedAt = 0;
let planForRun = [];
let doneSteps = 0;

function onProgress(p) {
  // The step name the backend reports moves the log on.
  const stepIndex = planForRun.findIndex((s) => s.step === p.step);
  if (stepIndex >= 0 && stepIndex > doneSteps) doneSteps = stepIndex;

  el.runningPhase.textContent = p.message || "";
  if (p.title) el.runningTitle.textContent = p.title;

  if (p.totalBytes > 0) {
    el.progressFill.classList.remove("is-indeterminate");
    const pct = Math.min(100, (p.doneBytes / p.totalBytes) * 100);
    el.progressFill.style.transform = `scaleX(${pct / 100})`;
    el.progressTrack.setAttribute("aria-valuenow", pct.toFixed(0));
    el.progressTrack.setAttribute(
      "aria-valuetext",
      `${p.message} ${formatBytes(p.doneBytes)} of ${formatBytes(p.totalBytes)}, ${pct.toFixed(0)}%`,
    );

    el.writtenDone.textContent = formatBytes(p.doneBytes).replace(/\s\w+$/, "");
    el.writtenTotal.textContent = ` / ${formatBytes(p.totalBytes)}`;

    // Measured, not assumed, the moment there is anything to measure.
    const elapsed = (performance.now() - runStartedAt) / 1000;
    if (elapsed > 2 && p.doneBytes > 0) {
      measuredRate = p.doneBytes / elapsed;
      el.rate.textContent = `${formatBytes(measuredRate)}/s`;
      const left = (p.totalBytes - p.doneBytes) / measuredRate;
      el.remaining.textContent = clock(left);
    }
  } else {
    // Steps without a byte count still need to look alive. The inline scale
    // has to go, or it would sit under the sweep's own transform.
    el.progressFill.style.transform = "";
    el.progressFill.classList.add("is-indeterminate");
    el.progressTrack.removeAttribute("aria-valuenow");
    el.progressTrack.setAttribute("aria-valuetext", p.message);
  }

  if (p.file) el.currentFile.textContent = p.file;
  renderLog(planForRun, doneSteps);
}

function buildRequest() {
  const shared = {
    drivePath: selectedDrive,
    formatDrive: on.format,
    formatFilesystem: filesystem(),
    formatLabel: driveLabelFor(cartridgeTitle(), filesystem()) || null,
    copyGame: on.copy,
    moveGame: on.move,
    closeSteam: on.closeSteam,
    verifyCopy: on.verify,
    trimAfterWrite: on.trim,
    writeIcon: on.icon,
    memoryCard: el.optMemcard.checked,
    platform: el.optPlatform.value,
  };

  if (isCollection()) {
    return {
      ...shared,
      title: collectionName(),
      executable: "",
      collectionCoverSource: art.cover?.path ?? null,
      collectionLogoSource: art.logo?.path ?? null,
      collectionIconSource: art.icon?.path ?? null,
      collectionBackgroundSource: art.background?.path ?? null,
      primaryGame: primaryIndexOf(picked),
      games: picked.map((g) => ({
        title: g.name,
        executable: g.executable,
        appId: g.library === "steam" ? g.id : null,
        playniteId: g.library === "playnite" ? g.id : null,
        // A scanned game has no launcher behind it, so its id *is* its folder.
        // The backend copies that folder and works out what to run inside it.
        sourceDir: g.library === "folder" ? g.id : null,
        coverSource: g.coverSource ?? null,
        backgroundSource: g.backgroundSource ?? null,
        logoSource: g.logoSource ?? null,
        iconSource: g.iconSource ?? null,
      })),
    };
  }

  if (manual) {
    return {
      ...shared,
      title: el.customTitle.value.trim(),
      // A ROM decides what Play opens once it is on the cartridge.
      executable: manual.rom ? "" : el.customExec.value.trim() || manual.executable || "",
      appId: null,
      playniteId: null,
      sourceDir: manual.folder?.path ?? null,
      romSource: manual.rom?.path ?? null,
      copyExecutable: manual.executable || null,
      coverSource: art.cover?.path ?? null,
      iconSource: art.icon?.path ?? null,
      logoSource: art.logo?.path ?? null,
      backgroundSource: art.background?.path ?? null,
    };
  }

  const game = picked[0];
  return {
    ...shared,
    title: game.name,
    executable: game.executable,
    appId: game.library === "steam" ? game.id : null,
    playniteId: game.library === "playnite" ? game.id : null,
    sourceDir: game.library === "folder" ? game.id : null,
    coverSource: art.cover?.path ?? null,
    iconSource: art.icon?.path ?? null,
    logoSource: art.logo?.path ?? null,
    backgroundSource: art.background?.path ?? null,
  };
}

/**
 * Ask before erasing, naming the drive and what it holds.
 *
 * Resolves true only for the button that says it will erase. Escape, the
 * cancel button and a click on the backdrop all mean no.
 */
function confirmErase() {
  const name = formatPlan?.currentLabel || driveLabel();
  el.confirmTitle.textContent = `Erase ${name}?`;
  el.confirmText.textContent = formatPlan?.warning ||
    `Everything on ${name} will be deleted.`;
  el.confirmDialog.returnValue = "";
  el.confirmDialog.showModal();
  return new Promise((resolve) => {
    el.confirmDialog.addEventListener(
      "close",
      () => resolve(el.confirmDialog.returnValue === "go"),
      { once: true },
    );
  });
}

/** Back to an empty build screen, keeping the drive. */
function startAnother() {
  finished = false;
  picked = [];
  manual = null;
  art.cover = null;
  art.logo = null;
  art.icon = null;
  art.background = null;
  el.collectionTitle.value = "";
  el.customTitle.value = "";
  el.customExec.value = "";
  el.search.value = "";
  resetRunOptions();
  status("");
  renderGames();
  renderTickCount();
  showPhase("build");
}

/** The one button: write, update, or start again, depending on the tab. */
function primaryAction() {
  if (activeTab === "created" || activeTab === "memcard") return;
  if (activeTab === "edit") return saveEdits();
  if (finished) return startAnother();
  return write();
}

async function write() {
  if (building || el.create.disabled) return;
  if (on.format && !(await confirmErase())) return;

  building = true;
  measuredRate = null;
  runStartedAt = performance.now();
  doneSteps = 0;

  // The plan the log ticks through, tagged with the step names the backend
  // reports so the two stay in step.
  planForRun = planSteps().map((step) => ({
    ...step,
    step: stepKeyFor(step.what),
  }));

  renderForced();
  el.railForced.hidden = false;
  el.railPlan.hidden = true;
  el.moveRow.hidden = true;
  el.runningTitle.textContent = cartridgeTitle();
  el.runningPhase.textContent = "Starting…";
  el.writtenDone.textContent = "—";
  el.writtenTotal.textContent = "";
  el.remaining.textContent = "—";
  el.rate.textContent = "";
  el.currentFile.textContent = "";
  el.progressFill.style.transform = "";
  el.progressFill.classList.add("is-indeterminate");
  renderLog(planForRun, 0);
  showPhase("running");
  refreshCreateButton();
  status("");
  el.close.disabled = true;

  try {
    const request = buildRequest();
    const result = await invoke("create_cartridge", { request });

    doneSteps = planForRun.length;
    renderLog(planForRun, doneSteps);
    el.progressFill.classList.remove("is-indeterminate");
    el.progressFill.style.transform = "scaleX(1)";

    // Formatting relabels the drive, and Linux remounts it under the new label:
    // the cartridge is wherever its cartridge.conf went, not where it started.
    if (result.confPath) {
      selectedDrive = result.confPath
        .replace(/[\\/]cartridge\.conf$/, "")
        .replace(/^([A-Za-z]:)$/, "$1\\"); // a Windows drive root keeps its backslash
    }
    const drive = drives.find((d) => d.path === selectedDrive);
    const parts = [
      `Cartridge written to ${drive ? drive.label : selectedDrive.split(/[\\/]/).pop()}.`,
    ];
    if (result.formatted) {
      parts.push(`Formatted to ${filesystemLabel(result.formattedFilesystem || request.formatFilesystem)}.`);
    }
    if (result.gameCopied) {
      parts.push(
        result.gameFolder
          ? `Copied ${formatBytes(result.bytesCopied)} to ${result.gameFolder}.`
          : `Copied ${formatBytes(result.bytesCopied)}.`,
      );
    }
    if (result.registeredWithSteam) parts.push("Registered as a Steam library.");
    for (const removed of result.removedFromPc ?? []) parts.push(`Moved: deleted ${removed}.`);
    for (const warning of result.warnings ?? []) parts.push(warning);
    // Asked for per write, so the next write starts from not deleting anything.
    moveWanted = false;

    // Tuning elevates, so it happens here rather than inside the copy: the UAC
    // prompt lands once, after the long part is over, and declining it costs
    // nothing that has already been written. Before the eject, because tuning a
    // cartridge that has just been unmounted does nothing at all.
    //
    // It was in the plan and in the summary before it was in the code: the run
    // showed "Tune Windows for this cartridge", finished, and reported "Windows
    // tuned" without anything having called apply_tuning. Which is why the
    // cartridges that had been through this wizard were still being scanned.
    if (on.tune) {
      try {
        const tuned = await invoke("apply_tuning", {
          drivePath: selectedDrive,
          tweaks: ["defender", "indexing"],
          applying: true,
        });
        parts.push(...tuned);
      } catch (error) {
        parts.push(`Written, but Windows was not tuned: ${error}`);
      }
    }

    // Ejecting is a step of the plan, not an afterthought, so it reports into
    // the same status line.
    if (on.eject) {
      try {
        await invoke("eject_drive", { drivePath: selectedDrive });
        parts.push("Safe to remove.");
      } catch (error) {
        parts.push(`Written, but the drive would not eject: ${error}`);
      }
    }

    status(parts.join(" "), (result.warnings ?? []).length ? "" : "good");
    el.runningPhase.textContent = "Done";
    if (result.bytesCopied) {
      el.writtenDone.textContent = formatBytes(result.bytesCopied);
      el.writtenTotal.textContent = "";
    }
    el.remaining.textContent = "0:00";
    finished = true;
    // Whatever the Edit tab had open may be the cartridge just overwritten.
    editing = null;
    // The backend put it on the shelf; the count in the sidebar should say so.
    void refreshCreated();
  } catch (error) {
    status(String(error), "error");
    el.runningPhase.textContent = "Stopped";
    showPhase("build");
  } finally {
    building = false;
    el.close.disabled = false;
    el.railForced.hidden = true;
    await refreshDrives();
    refreshCreateButton();
  }
}

/** Match a plan step to the step name the backend reports for it. */
function stepKeyFor(what) {
  const text = what.toLowerCase();
  if (text.startsWith("format")) return "format";
  if (text.startsWith("close steam")) return "steam";
  if (text.startsWith("copy")) return "copy";
  if (text.startsWith("verify")) return "verify";
  if (text.startsWith("delete")) return "move";
  if (text.startsWith("write")) return "autorun";
  if (text.startsWith("look up")) return "hltb";
  if (text.startsWith("tune")) return "tune";
  if (text.startsWith("release")) return "trim";
  if (text.startsWith("eject")) return "eject";
  return "";
}

/* ==========================================================================
   Artwork
   ========================================================================== */

let artTarget = "cover";
let sgdbTimer = null;

/**
 * Which game's poster is being chosen, as an index into `picked`.
 *
 * `null` means the cartridge's own art. A multicartridge has both: one
 * background, logo and icon for the cartridge, and a poster for each game on
 * it, because the launcher shows a different picture per row and a different
 * one again behind the whole thing.
 */
let artGame = null;

/**
 * The game a poster is being chosen for, from whichever list is on screen.
 *
 * Create orders `picked`; Edit orders the cartridge's own games. Resolving
 * against `picked` either way meant the poster button on the Edit tab pointed
 * into the wrong array — usually an empty one, so it silently did nothing.
 */
function artGameOf() {
  if (artGame === null) return null;
  const rows = activeTab === "edit" ? editing?.games ?? [] : picked;
  return rows[artGame] ?? null;
}

/** Redraw whichever list the poster just landed in. */
function renderActiveGameList() {
  if (activeTab === "edit") renderEditGames();
  else renderOrder();
}

function openArtwork(target, gameIndex = null) {
  artTarget = target;
  artGame = gameIndex;

  const game = artGameOf();
  // The same four slots whether this is the cartridge's art or one game's. A
  // game used to have exactly one picture, its poster, on the reasoning that
  // the other three belonged to the cartridge — which was true while the
  // launcher only ever showed a cover. A collection under a skin that wants
  // heroes needs a hero per game, so every game has the full set and any of
  // them may be left empty.
  el.sgdbTabs.hidden = false;
  for (const tab of el.sgdbTabs.children) {
    tab.setAttribute("aria-selected", String(tab.dataset.type === kindFor(artTarget)));
  }
  el.sgdbTitle.textContent = game ? `Artwork — ${game.name ?? game.title}` : "Artwork";
  el.sgdbSearch.value = artTitle();
  el.icoReceipt.hidden = true;
  refreshPreview();
  el.sgdbDialog.showModal();
  if (settings.steamgriddbEnabled) searchArtwork();
  else {
    el.sgdbStatus.textContent = "SteamGridDB search is off in Settings.";
    el.sgdbResults.replaceChildren();
  }
}

/** The name the artwork is for: the game, or whichever cartridge is open. */
function artTitle() {
  const game = artGameOf();
  if (game) return game.name ?? game.title ?? "";
  if (activeTab === "edit") return el.editTitle.value.trim() || editing?.title || "";
  return cartridgeTitle();
}

function kindFor(target) {
  return { cover: "grid", background: "hero", logo: "logo", icon: "icon" }[target] ?? "grid";
}

/**
 * How a downloaded image is filed.
 *
 * `cacheKey` names the file in the artwork cache, alongside a hash of the URL,
 * so it is namespaced by game as well as kind: two cartridges both asking for
 * a grid are not asking for the same picture. `gameKey` is what the backend
 * remembers the choice under, so reopening a cartridge can offer the artwork
 * that was used last time.
 */
function artworkKeys(target = artTarget) {
  const owner = artGameOf() ?? (isCollection() || manual ? null : picked[0]);
  const name = owner?.name ?? owner?.title ?? artTitle() ?? "";
  const game = name || "untitled";
  const keys = { cacheKey: `${game}-${kindFor(target)}` };
  // Only a cover is remembered as "this game's picture": the backend falls
  // back to it when a game has no cover of its own. Remembering the logo or
  // the icon under the same key made the last pick of any kind the cover.
  if (target === "cover") keys.gameKey = rememberKeyFor(owner, game);
  return keys;
}

/** The key the backend looks a game's remembered cover up by. */
function rememberKeyFor(game, fallback) {
  if (game?.library === "steam") return `steam:${game.id}`;
  if (game?.library === "playnite") return `playnite:${game.id}`;
  if (game?.library === "folder") {
    return String(game.id).split(/[\\/]/).filter(Boolean).pop() || fallback;
  }
  return fallback;
}

/**
 * File a chosen image against whatever the dialog is currently pointed at.
 *
 * A game's art lives on the game, so it survives reordering and so
 * buildRequest can hand the backend one set per `[game]` block. The
 * cartridge's own lives in `art`.
 */
function applyArt(target, game, path, preview) {
  if (game) {
    game[`${target}Source`] = path;
    game[target] = preview;
    renderActiveGameList();
    refreshRail();
    return;
  }
  art[target] = { path, preview };
  if (activeTab === "edit") renderEditArt();
}

/**
 * Which slot a pick belongs to, fixed at the moment it was clicked.
 *
 * A download takes a second or two, and reading the dialog's current tab when
 * it finished filed each picture under whichever tab was open by then: pick a
 * cover, move on to the logo, and the cover landed in the logo slot.
 */
function pickTarget() {
  return { target: artTarget, game: artGameOf() };
}

/** Downloads still in flight. Write waits for them rather than leave art off. */
let pendingArt = 0;

async function downloadArt(url) {
  const { target, game } = pickTarget();
  pendingArt += 1;
  refreshCreateButton();
  try {
    const got = await invoke("sgdb_download_artwork", { url, ...artworkKeys(target) });
    applyArt(target, game, got.path, got.dataUri);
    return { target, got };
  } finally {
    pendingArt -= 1;
    refreshCreateButton();
  }
}

function targetFor(kind) {
  // The other half of `kindFor`. The hero was missing from this one, so a Hero
  // slot opened the picker on a tab that did not exist: nothing showed as
  // selected, and every tab click landed back on the cover.
  return { grid: "cover", hero: "background", logo: "logo", icon: "icon" }[kind] ?? "cover";
}

// Clicking a picture in the preview is the same as clicking its tab.
el.sgdbPreview.addEventListener("click", (event) => {
  const pv = event.target.closest(".pv");
  if (!pv) return;
  const type = pv.dataset.type.split(" ")[0];
  el.sgdbTabs.querySelector(`[data-type="${type}"]`)?.click();
});

el.sgdbTabs.addEventListener("click", (event) => {
  const tab = event.target.closest(".tab");
  if (!tab) return;
  artTarget = targetFor(tab.dataset.type);
  for (const other of el.sgdbTabs.children) {
    other.setAttribute("aria-selected", String(other === tab));
  }
  el.icoReceipt.hidden = true;
  refreshPreview();
  if (settings.steamgriddbEnabled) searchArtwork();
});

async function searchArtwork() {
  const query = el.sgdbSearch.value.trim();
  if (!query) {
    el.sgdbStatus.textContent = "Type a game title.";
    return;
  }
  el.sgdbStatus.textContent = "Searching…";
  el.sgdbResults.replaceChildren();

  try {
    const found = await invoke("sgdb_search_games", { query });
    if (!found?.length) {
      el.sgdbStatus.textContent = "No SteamGridDB matches.";
      return;
    }
    const artwork = await invoke("sgdb_get_artwork", {
      gameId: found[0].id,
      artType: kindFor(artTarget),
    });
    renderArtwork(artwork ?? []);
  } catch (error) {
    el.sgdbStatus.textContent = String(error);
  }
}

function renderArtwork(list) {
  el.sgdbStatus.textContent = list.length ? "" : "No artwork of that kind.";
  el.sgdbResults.replaceChildren();

  for (const item of list) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "art-card";
    btn.setAttribute("aria-pressed", "false");

    const img = document.createElement("img");
    img.alt = "";
    img.loading = "lazy";
    img.src = safeRemoteSrc(item.thumb) || safeRemoteSrc(item.url);
    const meta = document.createElement("span");
    meta.className = "art-card__meta";
    meta.textContent = item.width && item.height ? `${item.width}×${item.height}` : "SteamGridDB";

    btn.append(img, meta);
    btn.addEventListener("click", () => chooseArtwork(item, btn));
    el.sgdbResults.append(btn);
  }
}

async function chooseArtwork(item, btn) {
  for (const other of el.sgdbResults.children) other.setAttribute("aria-pressed", "false");
  btn.setAttribute("aria-pressed", "true");
  el.sgdbStatus.textContent = "Fetching…";

  try {
    const { target, got } = await downloadArt(item.url);
    if (target === artTarget) el.sgdbStatus.textContent = "";

    // The icon is the only kind that becomes a different file on disk, so it
    // is the only one that reports back — a line of type, not a wizard page.
    if (target === "icon" && artTarget === "icon") {
      const name = String(got.path).split(/[/\\]/).pop();
      el.icoReceipt.hidden = false;
      el.icoReceipt.textContent = `${name} — becomes the drive icon.`;
    }

    refreshPreview();
    refreshRail();
  } catch (error) {
    el.sgdbStatus.textContent = String(error);
  }
}

/** Where the chosen artwork lands: the launcher, exactly as it will look. */
function refreshPreview() {
  // While a game's poster is being chosen, the preview shows that game — it is
  // the row the picture will land on, not the cartridge's own face.
  const targeted = artGameOf();
  // Each kind comes from the game when one is targeted, and from the cartridge
  // otherwise — a game carries all four of its own now. On the Edit tab what
  // the cartridge already has stands in until something new is chosen.
  const of = (kind) => {
    if (targeted) return targeted[kind];
    return art[kind]?.preview ?? (activeTab === "edit" ? editing?.[kind] : null);
  };

  const grid = of("cover") ?? (targeted || isCollection() || activeTab === "edit"
    ? null
    : picked[0]?.cover);

  // All four at once, each where it lands: the hero is its own banner rather
  // than standing in for the cover, which read as the hero being the cover.
  el.previewGrid.hidden = !safeSrc(grid);
  if (safeSrc(grid)) el.previewGrid.src = safeSrc(grid);
  el.previewArt.classList.toggle("has-art", Boolean(safeSrc(grid)));

  const hero = safeSrc(of("background"));
  el.previewHeroImg.hidden = !hero;
  if (hero) el.previewHeroImg.src = hero;

  // Mark the one the open tab is choosing.
  const kind = kindFor(artTarget);
  for (const pv of el.sgdbPreview.querySelectorAll(".pv")) {
    pv.classList.toggle("is-active", pv.dataset.type.split(" ").includes(kind));
  }
  el.previewIconLabel.textContent = platform === "windows" ? "In Explorer" : "In the file manager";

  const logo = of("logo");
  el.previewLogo.hidden = !safeSrc(logo);
  if (safeSrc(logo)) el.previewLogo.src = safeSrc(logo);
  el.previewStage.classList.toggle("has-logo", Boolean(safeSrc(logo)));
  el.previewTitle.textContent = artTitle() || "Nothing yet";

  const icon = of("icon") ?? grid;
  el.previewIcon.style.backgroundImage = safeSrc(icon) ? `url("${safeSrc(icon)}")` : "";
}

// The target outlives the dialog otherwise, and every later preview would go on
// showing the game whose poster was picked last.
el.sgdbDialog.addEventListener("close", () => {
  artGame = null;
  el.sgdbTabs.hidden = false;
  refreshPreview();
});

el.sgdbSearch.addEventListener("input", () => {
  clearTimeout(sgdbTimer);
  if (settings.steamgriddbEnabled) sgdbTimer = setTimeout(searchArtwork, 320);
});

el.sgdbUseManual.addEventListener("click", async () => {
  const url = el.sgdbManualUrl.value.trim();
  if (!url) return;
  el.sgdbStatus.textContent = "Fetching…";
  try {
    await downloadArt(url);
    el.sgdbStatus.textContent = "";
    refreshPreview();
    refreshRail();
  } catch (error) {
    el.sgdbStatus.textContent = String(error);
  }
});

/** A file on this machine, rather than anything fetched. */
async function pickCoverFile(target = artTarget) {
  // Read before the file dialog, which the art dialog may close under.
  const game = artGameOf();
  try {
    const chosen = await invoke("pick_cover_image");
    if (!chosen) return;
    if (game) {
      game[`${target}Source`] = chosen.path;
      game[target] = chosen.preview;
      renderActiveGameList();
      refreshPreview();
      refreshRail();
      return;
    }
    art[target] = { path: chosen.path, preview: chosen.preview };
    if (target === "cover") {
      setPlate(el.collectionCover, el.collectionCoverImg, chosen.preview);
    }
    refreshPreview();
    if (activeTab === "edit") renderEditArt();
    refreshRail();
  } catch (error) {
    status(String(error), "error");
  }
}

/* ==========================================================================
   Settings
   ========================================================================== */

function applySettings() {
  el.setSgdb.checked = Boolean(settings.steamgriddbEnabled);
  el.setSgdbKey.value = settings.steamgriddbApiKey ?? "";
  el.sgdbKeyField.hidden = !el.setSgdb.checked;
  el.setFilesystem.value = filesystemInfo(settings.defaultFilesystem)
    ? settings.defaultFilesystem
    : DEFAULT_FILESYSTEM;
  describeFilesystem();
  el.setVerify.checked = settings.defaultVerify !== false;
  el.setIcon.checked = settings.defaultIcon !== false;
  el.setEject.checked = settings.defaultEject !== false;
  el.setCopy.checked = settings.defaultCopy !== false;
  el.setCloseSteam.checked = settings.defaultCloseSteam !== false;
  el.setTune.checked = Boolean(settings.defaultTune);
  el.setTrim.checked = Boolean(settings.defaultTrim);
  el.setCopyRate.value = String(settings.defaultCopyRateMbS ?? 0);
  el.setOnInsert.value = settings.onCartridgeInsert || "focus_ui";
  describeOnInsert();
  // On unless it has been switched off: it writes to the cartridge only, and
  // only because Play was pressed.
  el.setPlaytime.checked = settings.trackPlaytime !== false;
  el.setLaunchOnTrigger.checked = Boolean(settings.launchOnTrigger);
  el.setIdle.value = String(settings.idlePauseMinutes ?? 10);
  el.setIdleRow.hidden = !el.setPlaytime.checked;
  el.setHltb.checked = Boolean(settings.hltbEnabled);
  el.setLudusavi.checked = Boolean(settings.ludusaviEnabled);
  // Off unless it has been switched on: it writes to the user's home.
  el.setSaveSync.checked = Boolean(settings.saveSync);
  // Tuning edits Defender and Search, which exist on one platform.
  el.setTuneRow.hidden = platform !== "windows";
  el.tuneNow.hidden = platform !== "windows";
  // Windows trims on its own schedule, so the switch would do nothing there.
  el.setTrimRow.hidden = platform === "windows";
  resetTune();
}

/** Settings changed, so anything derived from them is stale. */
function applyDefaults() {
  resetRunOptions();
  refreshRail();
}

/* ==========================================================================
   Create / Edit
   ========================================================================== */

const TABS = ["create", "edit", "created", "memcard", "actions"];

/**
 * Which panel is showing. The rail belongs to Create and Edit and never moves;
 * the shelf has no cartridge being built, so it hides the rail.
 */
async function showTab(name) {
  activeTab = TABS.includes(name) ? name : "create";
  const isEdit = activeTab === "edit";

  for (const [tab, panel, id] of [
    [el.tabCreate, el.panelCreate, "create"],
    [el.tabEdit, el.panelEdit, "edit"],
    [el.tabCreated, el.panelCreated, "created"],
    [el.tabMemcard, el.panelMemcard, "memcard"],
    [el.tabActions, el.panelActions, "actions"],
  ]) {
    const on = id === activeTab;
    tab.setAttribute("aria-selected", String(on));
    tab.tabIndex = on ? 0 : -1;
    panel.hidden = !on;
  }
  el.columns.classList.toggle(
    "is-shelf",
    activeTab === "created" || activeTab === "memcard" || activeTab === "actions",
  );

  stopListening();
  if (activeTab === "actions") {
    el.barText.textContent = "GamePak actions";
    await openActionsPage();
    return;
  }
  if (activeTab === "memcard") {
    el.barText.textContent = "Memory card";
    await openMemcardPage();
    return;
  }

  if (activeTab === "created") {
    el.barText.textContent = "Created cartridges";
    await refreshCreated();
    return;
  }

  if (isEdit) {
    await refreshEditDrives();
    const preferred = preferredCartridge();
    if (preferred && (!editing || editing.drivePath !== preferred.path)) {
      await openEditor(preferred.path);
    }
    el.barText.textContent = editing?.title ? `Edit ${editing.title}` : "Edit cartridge";
    // Coming back to a cartridge that is already open skips openEditor, and
    // without this the rail keeps whatever Create left in it — which looked
    // like switching tabs had thrown the preview away.
    refreshRail();
    return;
  }

  if (!building) await refreshDrives({ quick: true });
  el.barText.textContent = building
    ? `Writing ${on.format ? driveLabelFor(cartridgeTitle(), filesystem()) || driveLabel() : driveLabel()}`
    : isCollection()
      ? "Create multicartridge"
      : "Create cartridge";
  refreshRail();
}

/**
 * The cartridges that can be edited: every target drive that already holds one.
 *
 * This is the whole reason Edit is a tab. It used to be a link under the drive
 * list inside step 3, so reaching it meant ticking a game you did not want in
 * order to edit a cartridge that already existed.
 */
async function refreshEditDrives() {
  try {
    drives = await invoke("list_target_drives");
  } catch (error) {
    status(String(error), "error");
    return;
  }
  // Opening Edit with a cartridge plugged in and nothing selected picks one:
  // there is exactly one thing this tab can be about, and asking which is a
  // question with one answer.
  const preferred = preferredCartridge();
  if (activeTab === "edit" && preferred) {
    if (!editing || editing.drivePath !== preferred.path) {
      await openEditor(preferred.path);
    }
  }

  renderEditMedia();
}

/**
 * The cartridge Edit should be about: the one already open if it is still
 * plugged in, then the drive Create has selected, then any. The open one comes
 * first so that arriving from the shelf lands on the cartridge clicked there.
 */
function preferredCartridge() {
  const cartridges = (drives ?? []).filter((drive) => drive.hasCartridge);
  return cartridges.find((drive) => drive.path === editing?.drivePath)
    ?? cartridges.find((drive) => drive.path === selectedDrive)
    ?? cartridges[0];
}

/* ==========================================================================
   Created — the shelf this machine has written
   ========================================================================== */

/** Read the shelf and the drives, then draw both the list and the count. */
async function refreshCreated() {
  let shelf = [];
  try {
    shelf = (await invoke("list_created")) ?? [];
  } catch (error) {
    status(String(error), "error");
  }
  el.createdCount.textContent = shelf.length ? String(shelf.length) : "";
  el.createdCount.hidden = shelf.length === 0;
  if (activeTab !== "created") return;

  try {
    drives = await invoke("list_target_drives");
  } catch {
    // Without drives nothing shows as plugged in, which is still true enough.
  }
  renderCreated(shelf);
}

function renderCreated(shelf) {
  el.createdList.replaceChildren();
  el.createdEmpty.hidden = shelf.length > 0;

  for (const entry of shelf) {
    // A cartridge is known by the name in its conf; the volume label outlives
    // a rewrite, so it cannot say which cartridge is in the drive.
    const here = (drives ?? []).find(
      (drive) => drive.hasCartridge && drive.cartridgeTitle === entry.title,
    );

    const row = document.createElement("li");
    row.className = here ? "shelf-row is-here" : "shelf-row";

    const plate = document.createElement("div");
    plate.className = entry.cover ? "plate" : "plate plate--empty";
    if (entry.cover) {
      const img = document.createElement("img");
      img.alt = "";
      img.src = entry.cover;
      plate.append(img);
    }

    const body = document.createElement("div");
    body.className = "shelf-row__body";
    const title = document.createElement("p");
    title.className = "shelf-row__title";
    title.textContent = entry.title;
    const games = document.createElement("p");
    games.className = "shelf-row__games";
    // A single game is the title again; only a collection has a list to show.
    games.textContent = entry.games.length > 1
      ? `${entry.games.length} games · ${entry.games.join(", ")}`
      : entry.bytesCopied ? "Game on the cartridge" : "Game stays on this PC";
    const meta = document.createElement("p");
    meta.className = "shelf-row__meta mono";
    const parts = [
      entry.driveLabel,
      new Date(entry.writtenAt * 1000).toLocaleDateString(undefined, {
        day: "numeric", month: "short", year: "numeric",
      }),
    ];
    if (entry.bytesCopied) parts.push(formatBytes(entry.bytesCopied));
    meta.textContent = parts.filter(Boolean).join(" · ");
    if (here) {
      const plugged = document.createElement("span");
      plugged.className = "shelf-row__here";
      plugged.textContent = ` · plugged in at ${here.path}`;
      meta.append(plugged);
    }
    body.append(title, games, meta);
    row.append(plate, body);

    if (here) {
      const edit = document.createElement("button");
      edit.type = "button";
      edit.className = "change-btn";
      edit.textContent = "Edit";
      edit.addEventListener("click", async () => {
        await openEditor(here.path);
        await showTab("edit");
      });
      row.append(edit);
    }

    const forget = document.createElement("button");
    forget.type = "button";
    forget.className = "icon-btn shelf-row__forget";
    forget.setAttribute("aria-label", `Remove ${entry.title} from this list`);
    forget.title = "Remove from this list. The cartridge is not touched.";
    forget.innerHTML = '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" width="13" height="13" aria-hidden="true"><path d="M4.4 4.4l7.2 7.2M11.6 4.4l-7.2 7.2" /></svg>';
    forget.addEventListener("click", async () => {
      try {
        await invoke("forget_created", { id: entry.id });
      } catch (error) {
        status(String(error), "error");
      }
      await refreshCreated();
    });
    row.append(forget);

    el.createdList.append(row);
  }
}

/** The cartridge being edited, stated the way Create states its media. */
function renderEditMedia() {
  const drive = drives.find((d) => d.path === editing?.drivePath);
  if (drive) {
    el.editMediaTitle.textContent = editing.title || drive.label || drive.path;
    el.editMediaMeta.textContent =
      `${drive.label} · ${formatBytes(drive.freeBytes)} free of ${formatBytes(drive.totalBytes)}`;
  } else {
    el.editMediaTitle.textContent = "Choose a cartridge";
    el.editMediaMeta.textContent = "";
  }
}

/** Change, on the Edit tab: the same dialog, listing only written cartridges. */
function openCartridgePicker() {
  renderDrives({ onlyCartridges: true, choose: (drive) => openEditor(drive.path) });
  el.pickTitle.textContent = "Choose a cartridge";
  el.pickGames.hidden = true;
  el.pickMedia.hidden = false;
  el.pickDialog.showModal();
}

function openSettings() {
  applySettings();
  void renderFrontends();
  renderSources();
  el.settingsStatus.textContent = "";
  el.settingsDialog.showModal();
}

/**
 * What the library was built from.
 *
 * The backend reports problems it hit while scanning rather than a list of
 * enabled sources, so this says what was actually read and what was not — not
 * a set of switches the wizard cannot honour.
 */
function renderSources() {
  const counts = new Map();
  for (const game of library) {
    const key = game.source || game.library || "Unknown";
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  const parts = [...counts.entries()]
    .sort((a, b) => b[1] - a[1])
    .map(([name, n]) => `${name}: ${n}`);
  el.sources.textContent = parts.length
    ? `${library.length} games · ${parts.join(", ")}`
    : "No games found";

  const minutes = scannedAt ? Math.round((Date.now() - scannedAt) / 60000) : null;
  el.scanAge.textContent = minutes === null
    ? ""
    : minutes < 1
      ? "Scanned just now"
      : `Scanned ${minutes} minute${minutes === 1 ? "" : "s"} ago`;
}

async function saveSettings() {
  el.settingsStatus.textContent = "Saving…";
  // Start from what is on disk, not from what this window last read. The
  // front-end switches write themselves the moment they are flipped, and a
  // save built from a stale copy put every one of them back.
  let current = settings;
  try {
    current = (await invoke("get_settings")) ?? settings;
  } catch {
    // the copy this window has is the best there is
  }
  try {
    settings = await invoke("set_settings", {
      settings: {
        ...current,
        steamgriddbEnabled: el.setSgdb.checked,
        steamgriddbApiKey: el.setSgdbKey.value.trim(),
        defaultFilesystem: el.setFilesystem.value,
        defaultVerify: el.setVerify.checked,
        defaultIcon: el.setIcon.checked,
        defaultEject: el.setEject.checked,
        defaultCopy: el.setCopy.checked,
        defaultCloseSteam: el.setCloseSteam.checked,
        defaultTune: el.setTune.checked,
        defaultTrim: el.setTrim.checked,
        defaultCopyRateMbS: Number(el.setCopyRate.value) || 0,
        // Erasing is chosen per cartridge now, and never remembered.
        defaultFormat: false,
        onCartridgeInsert: el.setOnInsert.value,
        trackPlaytime: el.setPlaytime.checked,
        launchOnTrigger: el.setLaunchOnTrigger.checked,
        idlePauseMinutes: Number(el.setIdle.value) || 0,
        hltbEnabled: el.setHltb.checked,
        ludusaviEnabled: el.setLudusavi.checked,
        saveSync: el.setSaveSync.checked,
      },
    });
    applyDefaults();
    refreshRail();
    el.settingsDialog.close();
  } catch (error) {
    el.settingsStatus.textContent = String(error);
  }
}

/* ==========================================================================
   Editing a cartridge that is already there
   ========================================================================== */

async function openEditor(drivePath) {
  const target = drivePath ?? selectedDrive;
  if (!target) return;
  try {
    editing = await invoke("read_cartridge_for_edit", { drivePath: target });
  } catch (error) {
    status(String(error), "error");
    return;
  }
  // `art` is shared with Create, and nothing used to clear it: a cover chosen
  // while building a cartridge was still sitting there when Edit opened, and
  // Update wrote it to whatever cartridge happened to be selected. Editing
  // starts from what is on the drive, every time.
  art.cover = null;
  art.logo = null;
  art.icon = null;
  art.background = null;
  artGame = null;

  el.editTitle.value = editing.title ?? "";
  el.editMemcard.checked = Boolean(editing.memoryCard);
  fillPlatforms(el.editPlatform, editing.platform);
  renderEditArt();
  el.editStatus.textContent = "";
  el.editFormWrap.hidden = false;
  if (el.pickDialog.open) el.pickDialog.close();
  renderEditGames();
  renderEditMedia();
  refreshRail();
}

/** The three slots, showing what is chosen or what the cartridge already has. */
function renderEditArt() {
  // A collection's cover is written and then never shown: the launcher paints
  // the selected game's art, not the cartridge's. Until something displays it,
  // offering the slot is offering work with nowhere to land.
  const games = editing?.games ?? [];
  const collection = games.length > 1;
  el.editCoverSlot.hidden = collection;
  labelLogoSlot(el.editLogo, collection);

  setPlate(el.editCover, el.editCoverImg, art.cover?.preview ?? editing?.cover);
  const logo = art.logo?.remove ? null : art.logo?.preview ?? editing?.logo;
  setPlate(el.editLogo, el.editLogoImg, logo);
  el.btnRemoveLogo.hidden = !logo;
  el.btnRemoveLogo.textContent = collection ? "Remove collection logo" : "Remove logo";
  setPlate(el.editHero, el.editHeroImg, art.background?.preview ?? editing?.background);
  // A collection shows what saving will write: the primary game's. A single
  // game falls back to the cover only when it has no icon of its own, which
  // is what autorun.inf does with it.
  setPlate(
    el.editIcon,
    el.editIconImg,
    collection
      ? driveIconPreview(games, editing?.cover)
      : art.icon?.preview ?? editing?.icon ?? art.cover?.preview ?? editing?.cover,
  );
}

/**
 * Replace every poster on the cartridge with one fetched from SteamGridDB.
 *
 * Writes straight to the cartridge rather than staging like the rest of the
 * panel: it is a repair, and there is nothing to preview against — the point is
 * that what is there now is wrong.
 */
async function refetchArtwork() {
  if (!editing) return;
  if (!settings.steamgriddbEnabled || !(settings.steamgriddbApiKey ?? "").trim()) {
    el.editStatus.textContent =
      "SteamGridDB is switched off. Turn it on in Settings and add a key.";
    return;
  }
  el.btnRefetchArt.disabled = true;
  el.editStatus.textContent = "Fetching posters…";
  try {
    const result = await invoke("refetch_cartridge_artwork", { drivePath: editing.drivePath });
    await openEditor(editing.drivePath);
    el.editStatus.textContent = result.warnings?.length
      ? result.warnings.join(" ")
      : "Every game has a fresh poster.";
  } catch (error) {
    el.editStatus.textContent = String(error);
  } finally {
    el.btnRefetchArt.disabled = false;
  }
}

function renderEditGames() {
  renderGameRows(el.editGames, editing?.games ?? [], {
    removable: true,
    fallbackCover: editing?.isBundle ? "" : editing?.cover ?? "",
    onChange: () => {
      renderEditGames();
      renderEditArt();
      refreshRail();
    },
  });
}



/* --------------------------------------------------------------------------
   Tuning a cartridge that already exists

   Under the switch in Settings that says cartridges should be tuned, because
   that switch only governs cartridges not written yet — and the ones that need
   it are the ones already on the shelf, written while the create flow was
   reporting "Windows tuned" without calling anything. It also carries the
   reconciliation, so the exclusion an older cartridge left on a drive letter
   that has since moved is taken back from here too.

   It acts on the drive chosen on the Create tab. Tuning is per-volume and
   there is no such thing as tuning in general, so the alternative was picking a
   cartridge for the user or asking them twice.
   -------------------------------------------------------------------------- */

/** Which direction the shown plan is for, or null when none is shown. */
let tunePending = null;

/**
 * The drive tuning acts on: the one chosen on Create, else the cartridge open
 * on Edit, else the only drive plugged in.
 */
function tuneDrive() {
  const path = selectedDrive
    ?? (activeTab === "edit" ? editing?.drivePath : null)
    ?? (drives.length === 1 ? drives[0].path : null);
  return drives.find((d) => d.path === path) ?? null;
}

/** Name the drive on the buttons, so it is clear what they will change. */
function renderTuneTarget() {
  const drive = tuneDrive();
  el.btnTune.disabled = !drive;
  el.btnUntune.disabled = !drive;
  el.btnTune.textContent = drive ? `Apply to ${drive.label}` : "Apply to a drive";
  if (!drive && tunePending === null) {
    el.tuneStatus.textContent = drives.length
      ? "Choose a drive on the Create tab to apply this to it."
      : "Plug a cartridge in to apply this to it.";
  }
}

const TWEAKS = ["defender", "indexing"];

/**
 * Show the commands, and nothing else, until they are agreed to.
 *
 * This is elevated and one of the two commands weakens malware scanning, so it
 * is the commands themselves that get shown — not a description of them, and
 * not a checkbox that stands in for having read anything.
 */
async function showTunePlan(applying) {
  const drive = tuneDrive();
  if (!drive) {
    renderTuneTarget();
    return;
  }
  el.tuneStatus.textContent = "Reading what would change…";

  let commands;
  try {
    commands = await invoke("tuning_plan", {
      drivePath: drive.path,
      tweaks: TWEAKS,
      applying,
    });
  } catch (error) {
    el.tuneStatus.textContent = String(error);
    return;
  }

  tunePending = { applying, path: drive.path };
  el.tunePlan.replaceChildren();
  for (const command of commands) {
    const li = document.createElement("li");
    li.textContent = command;
    el.tunePlan.append(li);
  }
  el.tunePlan.hidden = commands.length === 0;
  el.tuneStatus.textContent = "Runs as administrator.";
  showTuneButtons(true);
}

function showTuneButtons(pending) {
  el.btnTune.hidden = pending;
  el.btnUntune.hidden = pending;
  el.btnTuneRun.hidden = !pending;
  el.btnTuneCancel.hidden = !pending;
}

function resetTune() {
  tunePending = null;
  el.tunePlan.hidden = true;
  el.tunePlan.replaceChildren();
  el.tuneStatus.textContent = "";
  showTuneButtons(false);
  renderTuneTarget();
}

async function runTuning() {
  if (tunePending === null) return;
  const { applying, path } = tunePending;
  el.btnTuneRun.disabled = true;
  el.tuneStatus.textContent = "Waiting for Windows…";

  try {
    const done = await invoke("apply_tuning", {
      drivePath: path,
      tweaks: TWEAKS,
      applying,
    });
    resetTune();
    el.tuneStatus.textContent = done.join(" ");
  } catch (error) {
    el.tuneStatus.textContent = String(error);
  } finally {
    el.btnTuneRun.disabled = false;
  }
}

el.btnTune.addEventListener("click", () => showTunePlan(true));
el.btnUntune.addEventListener("click", () => showTunePlan(false));
el.btnTuneRun.addEventListener("click", runTuning);
el.btnTuneCancel.addEventListener("click", resetTune);

async function saveEdits() {
  el.editStatus.textContent = "Saving…";
  try {
    await invoke("update_cartridge", {
      request: {
        drivePath: editing.drivePath,
        title: el.editTitle.value.trim() || editing.title,
        coverSource: art.cover?.path ?? null,
        logoSource: art.logo?.remove ? null : art.logo?.path ?? null,
        iconSource: art.icon?.path ?? null,
        backgroundSource: art.background?.path ?? null,
        primaryGame: primaryIndexOf(editing.games ?? []),
        removeLogo: Boolean(art.logo?.remove),
        memoryCard: el.editMemcard.checked,
        platform: el.editPlatform.value,
        games: (editing.games ?? []).map((g) => ({
          title: g.title,
          executable: g.executable,
          // Absent leaves whatever that game already had.
          coverSource: g.coverSource ?? null,
          backgroundSource: g.backgroundSource ?? null,
          logoSource: g.logoSource ?? null,
          iconSource: g.iconSource ?? null,
        })),
      },
    });
    el.editStatus.textContent = "";
    status("Cartridge updated. No files were moved.", "good");
    // Read it back so the panel shows what is on the drive rather than what
    // was typed, and so a second edit starts from the saved state.
    await openEditor(editing.drivePath);
    await refreshDrives();
  } catch (error) {
    el.editStatus.textContent = String(error);
  }
}

/* ==========================================================================
   Memory card — a drive of saves
   ==========================================================================
   Pick a drive, name it, add games. Each game needs to know where its saves
   live: Ludusavi's list is asked first when it is switched on, and a folder
   can always be pointed at by hand. A drive that is already a card opens with
   its games, so adding one does not mean entering the rest again. */

/** The games on the card being made: `CardGame` plus what the page shows. */
let mcwGames = [];

async function openMemcardPage() {
  await refreshDrives({ quick: true });
  const previous = el.mcwDrive.value;
  el.mcwDrive.replaceChildren(
    ...drives.map((drive) => {
      const option = document.createElement("option");
      option.value = drive.path;
      // The label already names the letter ("XCOM (D:)"), and a cartridge
      // named the same as its drive need not be said twice.
      const name = drive.label || drive.path;
      const title = drive.cartridgeTitle || "a cartridge";
      const what = drive.hasCartridge && !name.toLowerCase().startsWith(title.toLowerCase())
        ? ` — ${title}`
        : "";
      option.textContent = `${name}${what}`;
      return option;
    }),
  );
  if (!drives.length) {
    el.mcwDriveNote.textContent = "Plug a drive in.";
    mcwGames = [];
    renderMcwGames();
    return;
  }
  el.mcwDrive.value = drives.some((d) => d.path === previous) ? previous : drives[0].path;
  await loadMemcardDrive();
}

// ===========================================================================
// GamePak actions — a card or barcode, and what it starts
// ===========================================================================

// Per action type: the labels for settings a and b, null where it has none.
// "game" has no fields: it is picked from the installed games instead.
const ACTION_FIELDS = {
  moonlight: ["Moonlight host (name or address)", "App name"],
  remote: ["Gaming PC (as Moonlight names it)", "Game (as the gaming PC lists it)"],
  local: ["File path", null],
  exec: ["Program", "Arguments (optional, space-separated)"],
  open: ["URL", null],
  service: ["Service name", null],
};

let gamepaks = [];
let gpKind = null;
let gpGame = null;
let gpLearned = null;
let mlApps = [];

/** How an ID reads to a person. */
function describeTrigger(id) {
  if (id.startsWith("gp_nfc-")) return `NFC card ${id.slice(7)}`;
  if (/^gp_[0-9]+$/.test(id)) return `Barcode ${id.slice(3)}`;
  return id;
}

/** What a registered action starts, in a few words. */
function describeAction(action) {
  const base = (path) => String(path).split(/[\\/]/).pop();
  switch (action.type) {
    case "steam": {
      const game = library.find((g) => g.library === "steam" && g.id === String(action.appId));
      return game ? `Steam · ${game.name}` : `Steam game ${action.appId}`;
    }
    case "moonlight":
      return `Streams ${action.app} from ${action.target}`;
    case "remote":
      return `Game on ${action.moonlight}`;
    case "local":
      return `Opens ${base(action.path)}`;
    case "exec":
      return `Runs ${base(action.program)}`;
    case "open": {
      const game = library.find((g) => g.executable === action.target);
      return game ? `${game.source} · ${game.name}` : `Opens ${action.target}`;
    }
    case "service":
      return `Starts service ${action.target}`;
    case "cartridge":
      return `Cartridge · ${base(action.path)}`;
    default:
      return action.type;
  }
}

/** The GamePak an ID already starts, if any. */
function ownerOf(id) {
  return gamepaks.find((pak) => pak.id === id || pak.triggers.includes(id));
}

/**
 * Wait for the next trigger — a card tapped, a barcode scanned, anything that
 * runs `pc-gamepak --trigger` — and hand over its ID. The launcher leaves
 * every ID it is given in a file; this watches for that file to change.
 * Only one listener at a time: starting one stops the last.
 */
let stopListening = () => {};
function listenForTrigger(onId) {
  stopListening();
  let stopped = false;
  stopListening = () => {
    stopped = true;
  };
  (async () => {
    let before = (await invoke("last_trigger").catch(() => null))?.[1] ?? 0;
    while (!stopped) {
      await new Promise((resolve) => setTimeout(resolve, 400));
      const last = await invoke("last_trigger").catch(() => null);
      if (stopped || !last || last[1] === before) continue;
      before = last[1];
      if (onId(last[0]) !== "keep") stopped = true;
    }
  })();
}

// --- The list --------------------------------------------------------------

async function openActionsPage() {
  showGamePakHome();
  if (!library.length) await refreshLibrary();
  await refreshGamePaks();
}

function showGamePakHome() {
  stopListening();
  el.gpForm.hidden = true;
  el.gpHome.hidden = false;
}

async function refreshGamePaks() {
  try {
    gamepaks = await invoke("registered_gamepaks");
  } catch (error) {
    gamepaks = [];
    el.gpStatus.textContent = String(error);
  }
  el.gpList.replaceChildren(...gamepaks.map(gamePakRow));
  el.gpEmpty.hidden = gamepaks.length > 0;
}

function gamePakRow(pak) {
  const row = document.createElement("li");
  row.className = "shelf-row gp-row";

  const body = document.createElement("div");
  body.className = "shelf-row__body";
  const title = document.createElement("p");
  title.className = "shelf-row__title";
  title.textContent = pak.title || describeAction(pak.action);
  const what = document.createElement("p");
  what.className = "shelf-row__games";
  what.textContent = describeAction(pak.action);
  const chips = document.createElement("div");
  chips.className = "gp-chips";
  // Its own ID is a card's only when a person chose it; a generated one
  // (gp_ and 16 hex digits) is on no card.
  if (!/^gp_[0-9a-f]{16}$/.test(pak.id)) chips.append(triggerChip(pak.id));
  for (const trigger of pak.triggers) chips.append(triggerChip(trigger, pak));
  if (!chips.childElementCount) {
    const none = document.createElement("span");
    none.className = "gp-chip gp-chip--none";
    none.textContent = "No card yet";
    chips.append(none);
  }
  body.append(title, what, chips);

  const add = document.createElement("button");
  add.type = "button";
  add.className = "change-btn";
  add.textContent = "Add card";
  add.addEventListener("click", () => addCardTo(pak, row, add));

  const remove = document.createElement("button");
  remove.type = "button";
  remove.className = "icon-btn shelf-row__forget";
  remove.setAttribute("aria-label", `Delete ${pak.title || pak.id}`);
  remove.title = "Delete this GamePak. Its cards stop working here.";
  remove.innerHTML =
    '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" width="13" height="13" aria-hidden="true"><path d="M4.4 4.4l7.2 7.2M11.6 4.4l-7.2 7.2" /></svg>';
  // Two clicks: the first asks, the second deletes.
  remove.addEventListener("click", async () => {
    if (!row.classList.contains("is-confirming")) {
      row.classList.add("is-confirming");
      what.textContent = "Delete this GamePak? Click × again.";
      setTimeout(() => {
        row.classList.remove("is-confirming");
        what.textContent = describeAction(pak.action);
      }, 3000);
      return;
    }
    try {
      await invoke("remove_gamepak", { id: pak.id });
    } catch (error) {
      el.gpStatus.textContent = String(error);
    }
    await refreshGamePaks();
  });

  row.append(body, add, remove);
  return row;
}

/** A card or barcode that starts `pak`. Learned ones can be taken off. */
function triggerChip(id, pak) {
  const chip = document.createElement("span");
  chip.className = "gp-chip";
  chip.textContent = describeTrigger(id);
  chip.title = id;
  if (pak) {
    const off = document.createElement("button");
    off.type = "button";
    off.className = "gp-chip__off";
    off.setAttribute("aria-label", `Stop ${describeTrigger(id)} starting this`);
    off.textContent = "×";
    off.addEventListener("click", async () => {
      try {
        await invoke("remove_gamepak_trigger", { id: pak.id, trigger: id });
      } catch (error) {
        el.gpStatus.textContent = String(error);
      }
      await refreshGamePaks();
    });
    chip.append(off);
  }
  return chip;
}

/** One more card or barcode for a GamePak that exists. */
function addCardTo(pak, row, button) {
  for (const other of el.gpList.querySelectorAll(".is-listening")) {
    other.classList.remove("is-listening");
  }
  row.classList.add("is-listening");
  button.textContent = "Tap or scan…";
  el.gpStatus.textContent = `Tap a card or scan a barcode for ${pak.title || pak.id}.`;
  listenForTrigger((id) => {
    const owner = ownerOf(id);
    if (owner) {
      el.gpStatus.textContent =
        owner.id === pak.id
          ? `${describeTrigger(id)} already starts this.`
          : `${describeTrigger(id)} already starts ${owner.title || owner.id}.`;
      return "keep";
    }
    invoke("add_gamepak_trigger", { id: pak.id, trigger: id })
      .then(() => {
        el.gpStatus.textContent = `${describeTrigger(id)} now starts ${pak.title || pak.id}.`;
      })
      .catch((error) => {
        el.gpStatus.textContent = String(error);
      })
      .finally(refreshGamePaks);
  });
}

// --- New GamePak -------------------------------------------------------------

function openGamePakForm() {
  gpKind = null;
  gpGame = null;
  gpLearned = null;
  el.gpStatus.textContent = "";
  el.gpFormStatus.textContent = "";
  el.gpTitle.value = "";
  el.gpId.value = "";
  el.gpIdField.hidden = true;
  el.gpTypeId.hidden = false;
  el.gpTypeId.textContent = "Type an ID";
  el.gpGameSearch.value = "";
  mlApps = [];
  el.gpMlSearch.hidden = true;
  el.gpMlResults.replaceChildren();
  for (const input of [el.gpA, el.gpB]) input.value = "";
  chooseKind(null);
  setListen("waiting", "Waiting for a card or barcode…");
  el.gpHome.hidden = true;
  el.gpForm.hidden = false;
  listenForTrigger(learnedForNew);
  updateGamePakSave();
}

function setListen(state, text) {
  el.gpListen.dataset.state = state;
  el.gpListenText.textContent = text;
}

function learnedForNew(id) {
  const owner = ownerOf(id);
  if (owner) {
    setListen("waiting", `${describeTrigger(id)} already starts ${owner.title || owner.id}. Try another.`);
    return "keep";
  }
  gpLearned = id;
  setListen("learned", describeTrigger(id));
  el.gpTypeId.textContent = "Change";
  el.gpIdField.hidden = true;
  updateGamePakSave();
}

/** The ID typed by hand: a bare code — the number under a barcode — gets gp_. */
function typedId() {
  const id = el.gpId.value.trim().toLowerCase();
  if (!id) return null;
  return id.startsWith("gp_") ? id : `gp_${id}`;
}

function chooseKind(kind) {
  gpKind = kind;
  for (const button of el.gpKinds.querySelectorAll(".gp-kind")) {
    button.setAttribute("aria-checked", String(button.dataset.kind === kind));
  }
  el.gpGame.hidden = kind !== "game";
  el.gpMl.hidden = kind !== "moonlight" && kind !== "remote";
  if (kind === "moonlight" || kind === "remote") suggestMoonlightHosts();
  el.gpFields.hidden = !kind || kind === "game";
  if (kind === "game") renderGameResults();
  if (kind && kind !== "game") {
    const labels = ACTION_FIELDS[kind];
    for (const [label, field, text] of [
      [el.gpALabel, null, labels[0]],
      [el.gpBLabel, el.gpBField, labels[1]],
    ]) {
      label.textContent = text ?? "";
      if (field) field.hidden = !text;
    }
  }
  updateGamePakSave();
}

function renderGameResults() {
  const query = el.gpGameSearch.value.trim().toLowerCase();
  const matches = library
    .filter((game) => game.executable && game.name.toLowerCase().includes(query))
    .slice(0, 6);
  el.gpGameResults.replaceChildren(
    ...matches.map((game) => {
      const item = document.createElement("li");
      const button = document.createElement("button");
      button.type = "button";
      button.className = "mcw-result gp-result";
      button.setAttribute("aria-pressed", String(gpGame === game));
      button.innerHTML = "<span></span><span class=\"gp-result__source\"></span>";
      button.firstChild.textContent = game.name;
      button.lastChild.textContent = game.source;
      button.addEventListener("click", () => {
        gpGame = game;
        if (!el.gpTitle.value.trim() || el.gpTitle.dataset.auto === "1") {
          el.gpTitle.value = game.name;
          el.gpTitle.dataset.auto = "1";
        }
        renderGameResults();
        updateGamePakSave();
      });
      item.append(button);
      return item;
    }),
  );
  if (!matches.length) {
    const none = document.createElement("li");
    none.className = "hint";
    none.textContent = library.length ? "No installed game matches." : "No installed games found.";
    el.gpGameResults.append(none);
  }
}

/** Offer the hosts Moonlight is paired with for field A. Free text still works. */
async function suggestMoonlightHosts() {
  const hosts = await invoke("moonlight_hosts").catch(() => []);
  el.gpAList.replaceChildren(
    ...hosts.map((host) => Object.assign(document.createElement("option"), { value: host.name })),
  );
}

/** Ask Moonlight what the host in field A offers, and list it to pick from. */
async function findMoonlightApps() {
  el.gpMlFind.disabled = true;
  el.gpMlFind.textContent = "Asking the host…";
  try {
    mlApps = await invoke("moonlight_apps", { host: el.gpA.value });
    el.gpMlSearch.hidden = false;
    el.gpMlSearch.value = "";
    renderMoonlightApps();
    el.gpMlSearch.focus();
  } catch (error) {
    mlApps = [];
    el.gpMlResults.replaceChildren();
    el.gpFormStatus.textContent = String(error);
  } finally {
    el.gpMlFind.disabled = false;
    el.gpMlFind.textContent = "Find games on this host";
  }
}

function renderMoonlightApps() {
  const query = el.gpMlSearch.value.trim().toLowerCase();
  const matches = mlApps.filter((app) => app.toLowerCase().includes(query)).slice(0, 8);
  el.gpMlResults.replaceChildren(
    ...matches.map((app) => {
      const item = document.createElement("li");
      const button = document.createElement("button");
      button.type = "button";
      button.className = "mcw-result gp-result";
      button.setAttribute("aria-pressed", String(el.gpB.value === app));
      button.textContent = app;
      button.addEventListener("click", () => {
        el.gpB.value = app;
        if (!el.gpTitle.value.trim() || el.gpTitle.dataset.auto === "1") {
          el.gpTitle.value = app;
          el.gpTitle.dataset.auto = "1";
        }
        renderMoonlightApps();
        updateGamePakSave();
      });
      item.append(button);
      return item;
    }),
  );
}

/** The action to register, as add_gamepak takes it, or null if incomplete. */
function chosenAction() {
  if (gpKind === "game") {
    if (!gpGame) return null;
    if (gpGame.library === "steam") return { kind: "steam", a: gpGame.id };
    return gpGame.executable.includes("://")
      ? { kind: "open", a: gpGame.executable }
      : { kind: "exec", a: gpGame.executable };
  }
  if (!gpKind || !el.gpA.value.trim()) return null;
  const [, b] = ACTION_FIELDS[gpKind];
  return { kind: gpKind, a: el.gpA.value, b: b ? el.gpB.value : null };
}

function updateGamePakSave() {
  el.gpSave.disabled = !chosenAction() || !el.gpTitle.value.trim();
}

async function saveGamePak() {
  const action = chosenAction();
  if (!action) return;
  el.gpSave.disabled = true;
  el.gpFormStatus.textContent = "Saving…";
  const trigger = gpLearned ?? typedId();
  try {
    const id = await invoke("add_gamepak", {
      title: el.gpTitle.value,
      kind: action.kind,
      a: action.a,
      b: action.b ?? null,
      id: trigger,
    });
    if (trigger && trigger !== id) {
      await invoke("add_gamepak_trigger", { id, trigger });
    }
    showGamePakHome();
    await refreshGamePaks();
    el.gpStatus.textContent = trigger
      ? `Saved. ${describeTrigger(trigger)} now starts ${el.gpTitle.value.trim()}.`
      : `Saved as ${id}. Add a card to it to start it with a tap.`;
  } catch (error) {
    el.gpFormStatus.textContent = String(error);
    updateGamePakSave();
  }
}

/** Read what the chosen drive already is. */
async function loadMemcardDrive() {
  const drive = drives.find((d) => d.path === el.mcwDrive.value);
  el.mcwStatus.textContent = "";
  mcwGames = [];
  el.mcwName.value = "";
  // A cartridge takes a memory card beside it and becomes a combo drive; its
  // own file is not touched.
  el.mcwDriveNote.textContent = drive?.hasCartridge
    ? `${drive.cartridgeTitle || "This cartridge"} stays as it is. Writing adds a memory card beside it, so the drive is both.`
    : "";
  try {
    const games = (await invoke("memcard_games", { drivePath: drive.path })) ?? [];
    if (games.length) {
      const card = await invoke("memory_card", { drivePath: drive.path });
      el.mcwName.value = card?.title ?? "";
      el.mcwDriveNote.textContent = "Already has a memory card. Writing replaces its list with this one.";
    }
    mcwGames = games.map((game) => ({ ...game, state: "saved" }));
  } catch (error) {
    el.mcwStatus.textContent = String(error);
  }
  renderMcwGames();
}

/** Where the save goes on this machine, in the form it was written down. */
function mcwTemplateHere(game) {
  const own = platform === "windows" ? game.saveWindows : game.saveLinux;
  return own || game.save || "";
}

function renderMcwGames() {
  const drive = drives.find((d) => d.path === el.mcwDrive.value);
  el.mcwGamesEmpty.hidden = mcwGames.length > 0;
  el.mcwGames.replaceChildren(
    ...mcwGames.map((game, index) => {
      const li = document.createElement("li");
      li.className = "mcw-game";

      const plate = document.createElement("span");
      plate.className = game.icon ? "plate mcw-game__icon" : "plate plate--empty mcw-game__icon";
      if (safeSrc(game.icon)) {
        const img = document.createElement("img");
        img.alt = "";
        img.src = safeSrc(game.icon);
        plate.append(img);
      }

      const body = document.createElement("span");
      body.className = "mcw-game__body";
      const name = document.createElement("span");
      name.className = "mcw-game__name";
      name.textContent = game.title;
      const where = document.createElement("span");
      where.className = "mcw-game__where mono";
      const template = mcwTemplateHere(game);
      where.textContent = game.state === "looking"
        ? "Looking up where it saves…"
        : template || (game.note ?? "Where does it save? Choose the folder.");
      where.classList.toggle("is-missing", !template && game.state !== "looking");
      body.append(name, where);

      const pick = document.createElement("button");
      pick.type = "button";
      pick.className = "change-btn";
      pick.textContent = template ? "Change" : "Choose folder";
      pick.addEventListener("click", () => pickMcwFolder(index));

      const drop = document.createElement("button");
      drop.type = "button";
      drop.className = "row-btn row-btn--danger";
      drop.append(icon(ICON_CROSS));
      drop.title = `Take ${game.title} off the card`;
      drop.setAttribute("aria-label", drop.title);
      drop.addEventListener("click", () => {
        mcwGames.splice(index, 1);
        renderMcwGames();
      });

      li.append(plate, body, pick, drop);
      return li;
    }),
  );
  el.mcwWrite.disabled = !drive || !mcwGames.some(mcwTemplateHere);
}

function renderMcwResults() {
  const query = el.mcwSearch.value.trim().toLowerCase();
  el.mcwResults.replaceChildren();
  if (!query) return;
  const taken = new Set(mcwGames.map((game) => game.title.toLowerCase()));
  const matches = library
    .filter((game) => game.name.toLowerCase().includes(query) && !taken.has(game.name.toLowerCase()))
    .slice(0, 6);
  const add = (label, onClick) => {
    const li = document.createElement("li");
    const button = document.createElement("button");
    button.type = "button";
    button.className = "mcw-result";
    button.textContent = label;
    button.addEventListener("click", onClick);
    li.append(button);
    el.mcwResults.append(li);
  };
  for (const game of matches) add(game.name, () => addMcwGame(game));
  // A game the library does not know — an emulator, something run from a
  // folder — can still carry a save; it just has to be named.
  const typed = el.mcwSearch.value.trim();
  if (!matches.some((game) => game.name.toLowerCase() === query)) {
    add(`Add “${typed}” by name`, () => addMcwGame({ name: typed }));
  }
}

async function addMcwGame(game) {
  const entry = {
    title: game.name,
    executable: game.executable ?? "",
    appId: game.library === "steam" ? game.id : null,
    playniteId: game.library === "playnite" ? game.id : null,
    icon: game.cover ?? "",
    state: settings.ludusaviEnabled ? "looking" : "manual",
  };
  mcwGames.push(entry);
  el.mcwSearch.value = "";
  renderMcwResults();
  renderMcwGames();

  if (!entry.icon && game.library && game.id) {
    invoke("game_cover", { library: game.library, id: game.id })
      .then((cover) => {
        entry.icon = cover;
        renderMcwGames();
      })
      .catch(() => {});
  }
  if (!settings.ludusaviEnabled) {
    entry.note = "Choose the folder it saves into, or switch on Ludusavi in Settings.";
    renderMcwGames();
    return;
  }
  try {
    const found = await invoke("lookup_save_location", {
      title: entry.title,
      executable: entry.executable,
    });
    entry.saveWindows = found?.windows ?? null;
    entry.saveLinux = found?.linux ?? null;
    entry.state = "found";
    if (!found) entry.note = "Ludusavi does not know this one. Choose the folder it saves into.";
    else if (!mcwTemplateHere(entry)) entry.note = "Ludusavi knows it, but not for this system. Choose the folder.";
  } catch (error) {
    entry.state = "manual";
    entry.note = String(error);
  }
  renderMcwGames();
}

async function pickMcwFolder(index) {
  const game = mcwGames[index];
  let picked;
  try {
    picked = await invoke("pick_save_folder");
  } catch (error) {
    el.mcwStatus.textContent = String(error);
    return;
  }
  if (!picked) return;
  if (!picked.template) {
    game.note = "That folder is outside your user folders, so another PC could not find it. Choose the one the game saves into under your user folder.";
    renderMcwGames();
    return;
  }
  // Picked by hand is right by definition, on every system it can be read on.
  game.save = picked.template;
  game.saveWindows = null;
  game.saveLinux = null;
  game.note = "";
  renderMcwGames();
}

async function writeMemoryCard() {
  const drive = drives.find((d) => d.path === el.mcwDrive.value);
  if (!drive) return;
  el.mcwWrite.disabled = true;
  el.mcwStatus.textContent = "Writing…";
  try {
    const warnings = await invoke("create_memory_card", {
      request: {
        drivePath: drive.path,
        title: el.mcwName.value.trim() || "Memory card",
        games: mcwGames.map((game) => ({
          title: game.title,
          executable: game.executable ?? "",
          appId: game.appId ?? null,
          playniteId: game.playniteId ?? null,
          iconSource: game.iconSource ?? null,
          save: game.save ?? null,
          saveWindows: game.saveWindows ?? null,
          saveLinux: game.saveLinux ?? null,
        })),
      },
    });
    status(`Memory card written to ${drive.label || drive.path}.`, "good");
    // Read back first: loading the drive clears the line this is about to set.
    await loadMemcardDrive();
    el.mcwStatus.textContent = (warnings ?? []).join(" ");
  } catch (error) {
    el.mcwStatus.textContent = String(error);
  } finally {
    renderMcwGames();
  }
}

el.mcwDrive.addEventListener("change", loadMemcardDrive);
el.mcwSearch.addEventListener("input", renderMcwResults);
el.mcwWrite.addEventListener("click", writeMemoryCard);

/* ==========================================================================
   Wiring
   ========================================================================== */

el.search.addEventListener("input", renderGames);

el.changeGame.addEventListener("click", () => (manual ? enterManual() : openGamePicker()));
el.changeMedia.addEventListener("click", openMediaPicker);
// method="dialog" means a stray Enter in the search box would close the dialog
// mid-selection, which is the opposite of what Enter means in a filter. Only a
// button that means to close one gets to.
const closesDialog = (event, ...ids) => {
  if (!ids.includes(event.submitter?.id)) event.preventDefault();
};
el.pickForm.addEventListener("submit", (e) => closesDialog(e, "pick-close", "pick-done"));
el.sgdbForm.addEventListener("submit", (e) => closesDialog(e, "sgdb-close", "sgdb-done"));
// Entering a game by hand is an answer to "which game", so the dialog that
// asked has done its job.
el.btnCustom.addEventListener("click", () => {
  if (el.pickDialog.open) el.pickDialog.close();
  enterManual();
});

// Each slot opens the artwork picker on its own kind. 8a's "click one to change
// it" — the slot is the control, not a label beside one.
el.artSlots.addEventListener("click", (event) => {
  const slot = event.target.closest(".art-slot");
  if (slot) openArtwork(slot.dataset.slot);
});
el.btnCustomBack.addEventListener("click", () => {
  manual = null;
  showPhase("build");
  renderTickCount();
  refreshRail();
});
// Keeps what was entered and goes back to the build screen, where the drive
// and the options are. The card there reopens it.
el.btnCustomDone.addEventListener("click", () => {
  if (manual) manual.title = el.customTitle.value.trim();
  showPhase("build");
  refreshRail();
});
el.btnPickFolder.addEventListener("click", pickFolder);
el.btnPickRom.addEventListener("click", pickRom);
el.btnClearRom.addEventListener("click", () => {
  if (!manual) return;
  manual.rom = null;
  renderRom();
  refreshRail();
  refreshCreateButton();
});
el.btnClearFolder.addEventListener("click", () => {
  if (!manual) return;
  manual.folder = null;
  manual.choices = [];
  manual.executable = "";
  renderFolder();
  renderExeChoices();
  refreshRail();
});
el.customTitle.addEventListener("input", () => {
  refreshRail();
  refreshCreateButton();
});
el.customExec.addEventListener("input", () => {
  if (manual && el.customExec.value.trim()) manual.executable = "";
  renderExeChoices();
  refreshCreateButton();
});
el.btnCustomCover.addEventListener("click", () => pickCoverFile("cover"));

el.sgdbUseLocal.addEventListener("click", () => pickCoverFile(artTarget));
el.btnInheritCover.addEventListener("click", async () => {
  const first = picked[0];
  if (!first) return;
  try {
    const cover = await invoke("game_cover", { library: first.library, id: first.id });
    if (cover) {
      art.cover = { path: null, preview: cover, inherited: first };
      setPlate(el.collectionCover, el.collectionCoverImg, cover);
      refreshRail();
    }
  } catch {
    // nothing cached for it either
  }
});


el.create.addEventListener("click", primaryAction);
el.tabCreate.addEventListener("click", () => showTab("create"));
el.tabEdit.addEventListener("click", () => showTab("edit"));
el.tabCreated.addEventListener("click", () => showTab("created"));
el.tabMemcard.addEventListener("click", () => showTab("memcard"));
el.tabActions.addEventListener("click", () => showTab("actions"));
el.gpNew.addEventListener("click", openGamePakForm);
el.gpCancel.addEventListener("click", showGamePakHome);
el.gpSave.addEventListener("click", saveGamePak);
el.gpTypeId.addEventListener("click", () => {
  // Learned already: forget it and listen for another.
  if (gpLearned) {
    gpLearned = null;
    el.gpTypeId.textContent = "Type an ID";
    setListen("waiting", "Waiting for a card or barcode…");
    listenForTrigger(learnedForNew);
    return;
  }
  el.gpIdField.hidden = false;
  el.gpTypeId.hidden = true;
  el.gpId.focus();
});
el.gpKinds.addEventListener("click", (event) => {
  const kind = event.target.closest(".gp-kind")?.dataset.kind;
  if (kind) chooseKind(kind);
});
el.gpGameSearch.addEventListener("input", renderGameResults);
el.gpMlFind.addEventListener("click", findMoonlightApps);
el.gpMlSearch.addEventListener("input", renderMoonlightApps);
for (const input of [el.gpA, el.gpB]) {
  input.addEventListener("input", updateGamePakSave);
}
el.gpTitle.addEventListener("input", () => {
  el.gpTitle.dataset.auto = "0";
  updateGamePakSave();
});
// Drives are read again when the picker opens and when Create is shown, not on
// window focus. A refresh on focus took the foreground back from a launcher
// that had just opened on top of the wizard, so its arrow keys went here.
// A vertical tablist moves with up and down, as a screen reader announces it.
$("sidebar").addEventListener("keydown", (event) => {
  const step = { ArrowDown: 1, ArrowUp: -1 }[event.key];
  if (!step) return;
  event.preventDefault();
  const next = TABS[(TABS.indexOf(activeTab) + step + TABS.length) % TABS.length];
  void showTab(next).then(() => $(`tab-${next}`).focus());
});
el.btnRefetchArt.addEventListener("click", refetchArtwork);
// Staged like every other change here, and written on Update.
el.btnRemoveLogo.addEventListener("click", () => {
  art.logo = { remove: true };
  renderEditArt();
  refreshRail();
});

// Edit is now a single tab action, not a second link in the drive list.
draggable(el.editGames, () => editing?.games ?? [], () => {
  renderEditGames();
  refreshRail();
});

el.btnChangeCart.addEventListener("click", openCartridgePicker);
el.editArtSlots.addEventListener("click", (event) => {
  const slot = event.target.closest(".art-slot");
  if (slot) openArtwork(slot.dataset.slot);
});
// The preview is the reason to type a name here, so it keeps up with the typing.
el.editTitle.addEventListener("input", refreshRail);
/** Set once the plan has been shown, so the second press is the one that acts. */
let steamConfirmed = false;

// Shown before it runs. Registering the cartridge is harmless, but the other
// half moves a manifest out of another Steam library, and a game library is not
// somewhere to make silent changes.
el.btnRegister.addEventListener("click", async () => {
  if (!steamConfirmed) {
    steamConfirmed = true;
    el.btnRegister.textContent = "Do it";
    status(steamPlan.join(" "), "");
    return;
  }

  try {
    await invoke("register_with_steam", { drivePath: selectedDrive });
    driveIsSteamLibrary = true;
    steamPlan = [];
    steamConfirmed = false;
    el.btnRegister.hidden = true;
    el.btnUnregister.hidden = false;
    status("Done. Start Steam and the game is on the cartridge.", "good");
  } catch (error) {
    steamConfirmed = false;
    el.btnRegister.textContent = "Fix this cartridge in Steam";
    status(String(error), "error");
  }
});

el.btnUnregister.addEventListener("click", async () => {
  try {
    await invoke("unregister_from_steam", { drivePath: selectedDrive });
    driveIsSteamLibrary = false;
    el.btnUnregister.hidden = true;
    status("Removed from Steam's library list.", "good");
  } catch (error) {
    status(String(error), "error");
  }
});

el.settings.addEventListener("click", openSettings);
el.settingsSave.addEventListener("click", saveSettings);
/**
 * Say what the chosen reaction actually does, including where it will not.
 *
 * Each of the three non-default choices has a limit worth knowing before it is
 * picked rather than after it fails to happen, and two of them are the kind of
 * thing that reads as a bug when it is a deliberate refusal.
 */
function describeOnInsert() {
  const hints = {
    focus_ui: "The cartridge's window opens and comes to the front.",
    auto_launch_game:
      "Starts the game without a window — for a cartridge that points at a game " +
      "your PC already has. A game stored on the cartridge still waits for a " +
      "click, because a drive someone handed you should not get to run a program " +
      "on its own.",
    notify_only:
      "A notification, and nothing else. If one cannot be posted, the launcher " +
      "opens instead.",
    none: "Nothing happens. The tray and the desktop entry still open it.",
  };
  el.onInsertHint.textContent = hints[el.setOnInsert.value] ?? "";
  el.onInsertHint.hidden = !el.onInsertHint.textContent;
}

/**
 * Say what each filesystem costs, where the choice is made.
 *
 * The consequences are not guessable from the name and they are not small: one
 * of the three cannot hold a symlink, which is what Steam needs 1,892 of to put
 * Proton on a cartridge, and another cannot be read by Windows at all.
 */
/**
 * Draw the front-ends, each with its own switch.
 *
 * Built from what the backend knows rather than hardcoded here, so a front-end
 * added later appears without this file changing. Three states matter and are
 * all shown: switched on, installed, and implemented at all — a switch for a
 * plugin that is not on the machine would be a switch that does nothing, and one
 * for a plugin nobody has written yet would be worse.
 */
async function renderFrontends() {
  let list = [];
  try {
    list = (await invoke("frontends")) ?? [];
  } catch (error) {
    el.frontends.textContent = "Could not read which front-ends are set up.";
    console.error("[pc-gamepak]", error);
    return;
  }

  el.frontends.replaceChildren();
  for (const front of list) {
    const row = document.createElement("label");
    row.className = "opt";

    const box = document.createElement("input");
    box.type = "checkbox";
    box.id = `frontend-${front.id}`;
    box.checked = Boolean(front.on);
    // Not installed, or not written: the switch would do nothing either way.
    box.disabled = !front.implemented || !front.installed;
    box.addEventListener("change", () => {
      void invoke("set_frontend", { id: front.id, on: box.checked })
        .then(() => renderFrontends())
        .catch((error) => {
          box.checked = !box.checked;
          console.error("[pc-gamepak]", error);
        });
    });

    const body = document.createElement("span");
    body.className = "opt__body";
    const label = document.createElement("span");
    label.className = "opt__label";
    label.textContent = front.name;
    const hint = document.createElement("span");
    hint.className = "opt__hint";
    hint.textContent = frontendHint(front);
    body.append(label);
    if (hint.textContent) body.append(hint);
    row.append(box, body);
    el.frontends.append(row);
  }

  // Everything off is allowed — somebody may want a cartridge that waits to be
  // opened — and is worth saying out loud, because it is rarely the intention.
  const anyOn = list.some((front) => front.on);
  el.frontendsWarning.hidden = anyOn;
  el.frontendsWarning.textContent = anyOn ? "" : "Nothing opens on insert.";
}

/** What to say under a front-end's name. */
function frontendHint(front) {
  if (!front.implemented) return "Coming later";
  if (!front.installed) return "Not installed";
  return "";
}

/**
 * What a cartridge can say it is for (`platform=`), as cartridge.conf.example
 * lists them. A front-end that draws the physical cartridge picks its shell
 * from this; the launcher itself only hands it to skins.
 */
const PLATFORMS = [
  ["PC", "PC"], ["DOS", "DOS"],
  ["NES", "NES"], ["SNES", "Super Nintendo"], ["N64", "Nintendo 64"], ["GameCube", "GameCube"],
  ["Wii", "Wii"], ["WiiU", "Wii U"], ["Switch", "Switch"],
  ["GB", "Game Boy"], ["GBC", "Game Boy Color"], ["GBA", "Game Boy Advance"], ["NDS", "Nintendo DS"],
  ["3DS", "Nintendo 3DS"], ["VirtualBoy", "Virtual Boy"],
  ["MasterSystem", "Master System"], ["Genesis", "Mega Drive / Genesis"], ["SegaCD", "Mega-CD"],
  ["32X", "32X"], ["Saturn", "Saturn"], ["Dreamcast", "Dreamcast"], ["GameGear", "Game Gear"],
  ["PS1", "PlayStation"], ["PS2", "PlayStation 2"], ["PS3", "PlayStation 3"], ["PSP", "PSP"], ["Vita", "PS Vita"],
  ["Xbox", "Xbox"], ["Xbox360", "Xbox 360"],
  ["Atari2600", "Atari 2600"], ["Atari7800", "Atari 7800"], ["Lynx", "Lynx"], ["Jaguar", "Jaguar"],
  ["TurboGrafx16", "TurboGrafx-16"], ["NeoGeo", "Neo Geo"], ["NeoGeoPocket", "Neo Geo Pocket"],
  ["WonderSwan", "WonderSwan"], ["Arcade", "Arcade"], ["C64", "Commodore 64"], ["Amiga", "Amiga"], ["MSX", "MSX"],
];

/** Fill a platform picker, keeping a value this list does not know. */
function fillPlatforms(select, current = "PC") {
  const known = PLATFORMS.some(([id]) => id.toLowerCase() === String(current).toLowerCase());
  const entries = known ? PLATFORMS : [...PLATFORMS, [current, current]];
  select.replaceChildren(
    ...entries.map(([id, name]) => {
      const option = document.createElement("option");
      option.value = id;
      option.textContent = name;
      return option;
    }),
  );
  select.value = entries.find(([id]) => id.toLowerCase() === String(current || "PC").toLowerCase())?.[0] ?? "PC";
}
fillPlatforms(el.optPlatform);
el.optPlatform.addEventListener("change", () => {
  refreshRail();
  refreshCreateButton();
});

el.optMove.addEventListener("change", () => {
  moveWanted = el.optMove.checked;
  refreshPlan();
});

el.setOnInsert.addEventListener("change", describeOnInsert);
el.setPlaytime.addEventListener("change", () => {
  el.setIdleRow.hidden = !el.setPlaytime.checked;
});

el.setSgdb.addEventListener("change", () => {
  el.sgdbKeyField.hidden = !el.setSgdb.checked;
});
el.setFilesystem.addEventListener("change", describeFilesystem);
el.btnRescan.addEventListener("click", async () => {
  await refreshLibrary();
  await refreshDrives();
  renderSources();
});

/**
 * What "Tune Windows for this cartridge" covers, under the names the backend
 * parses. One checkbox, two tweaks — Defender and Search, as the step it adds
 * to the plan says.
 */
const WINDOWS_TWEAKS = ["defender", "indexing"];


el.close.addEventListener("click", async () => {
  if (building) return;
  if (tauri?.window) await tauri.window.getCurrentWindow().close();
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    if (building) event.preventDefault();
    return;
  }
  if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
    event.preventDefault();
    if (!el.create.disabled && !document.querySelector("dialog[open]")) primaryAction();
  }
});

/* ==========================================================================
   Boot
   ========================================================================== */

async function refreshLibrary() {
  try {
    const result = await invoke("list_games");
    library = result.games ?? [];
    scannedAt = Date.now();
    // A scanner that found nothing is not an error the person making a
    // cartridge can act on — Playnite not having a JSON export is a fact about
    // Playnite. The games it did find are the answer; the rest goes to the
    // console for whoever is debugging a scanner.
    if (result.problems?.length) console.info("[gamepak] scan:", result.problems.join(" "));
  } catch (error) {
    library = [];
    status(String(error), "error");
  }
  renderGames();
}

/**
 * Read the drives again. Drives come and go while the wizard is open, and this
 * used to run only at startup and after a write — so a drive plugged in later
 * never reached Create, until visiting Edit happened to read the list for it.
 *
 * `quick` skips the volumes with no letter, which on Windows means a
 * PowerShell round trip: fine when the picker opens, too slow for every focus.
 */
async function refreshDrives({ quick = false } = {}) {
  try {
    drives = await invoke("list_target_drives");
  } catch {
    drives = [];
  }
  if (!quick) {
    try {
      unmounted = await invoke("list_unmounted_volumes");
    } catch {
      unmounted = [];
    }
  }
  // A drive that has gone away takes the selection with it.
  if (selectedDrive && !drives.some((d) => d.path === selectedDrive)) {
    selectedDrive = null;
    formatPlan = null;
  }
  renderDrives();
  if (!selectedDrive && drives.length === 1) await selectDrive(drives[0]);
  refreshRail();
}

async function start() {
  // Subscribed before the first await, not after. open_wizard emits
  // "open-settings" as soon as the page finishes loading, and a listener
  // attached after the library scan would arrive long after that event had come
  // and gone — opening the wizard on the game list instead of on Settings.
  //
  // Showing the window is deliberately not done here; open_wizard does it on
  // page load, so nothing below can strand a window nobody can see.
  if (tauri?.event) {
    tauri.event.listen("open-settings", openSettings);
    tauri.event.listen("cartridge://progress", (event) => onProgress(event.payload));
  }

  try {
    platform = await invoke("host_platform");
  } catch {
    platform = "";
  }

  try {
    settings = { ...settings, ...(await invoke("get_settings")) };
  } catch {
    // defaults stand
  }
  // Before applyDefaults, because the label the wizard proposes depends on the
  // chosen filesystem's limit, and that limit now comes from this list.
  await loadFilesystems();
  applyDefaults();

  await refreshLibrary();
  await refreshDrives();
  refreshRail();
  showPhase("build");
  void refreshCreated();
  // The browser preview can open on any panel: `?tab=created`.
  if (!tauri) {
    const tab = new URLSearchParams(location.search).get("tab");
    if (tab) await showTab(tab);
  }
}

/* ==========================================================================
   Browser preview
   --------------------------------------------------------------------------
   Outside Tauri the wizard serves a sample library so the window can be
   designed and reviewed without a drive:
       npx http-server tauri-ui  →  http://localhost:8080/app/create.html
   ========================================================================== */

/** The browser preview's GamePak registry, kept as the real one is. */
let demoGamePaks = [
  { id: "gp_stardew", title: "Stardew Valley", action: { type: "steam", appId: 413150 },
    triggers: ["gp_nfc-4726aa64", "gp_3501170818305"] },
  { id: "gp_0123456789abcdef", title: "Movie night", triggers: [],
    action: { type: "local", path: "/media/films/The Matrix.mkv" } },
];

/** An action as add_gamepak's settings a and b describe it, as the registry stores it. */
function demoAction(kind, a, b) {
  switch (kind) {
    case "steam": return { type: "steam", appId: Number(a) };
    case "moonlight": return { type: "moonlight", target: a, app: b };
    case "remote": return { type: "remote", moonlight: a, app: b };
    case "local": return { type: "local", path: a };
    case "exec": return { type: "exec", program: a };
    default: return { type: kind, target: a };
  }
}

/** The browser preview's shelf. One cartridge plugged in, one on the shelf. */
let demoShelf = [
  { id: "1758900000", title: "God of War Collection",
    games: ["God of War (2018)", "God of War Ragnarök"], driveLabel: "HOLLOW",
    writtenAt: 1758900000, bytesCopied: 190_000_000_000, cover: "src/demo/gow-collection.jpg" },
  { id: "1758200000", title: "Hollow Knight", games: ["Hollow Knight"], driveLabel: "KNIGHT",
    writtenAt: 1758200000, bytesCopied: 0, cover: "src/demo/cover.jpg" },
];

let demoTapAt;

async function demoInvoke(command, args) {
  switch (command) {
    // Kept in step with core::format by hand, which is the price of a preview
    // that runs with no backend at all. Only the fields the picker reads.
    case "list_filesystems":
      return [
        { id: "ntfs", name: "NTFS", labelLimit: 32, canCreateHere: true, runsProton: true,
          nativeOn: ["Windows", "Linux"], needs: "",
          summary: "Reads natively on Windows and Linux, and Proton works." },
        { id: "exfat", name: "exFAT", labelLimit: 11, canCreateHere: true, runsProton: false,
          nativeOn: ["Windows", "Linux", "macOS"], needs: "",
          summary: "Reads and writes everywhere, macOS included, and holds no symlinks." },
        { id: "btrfs", name: "btrfs", labelLimit: 256, canCreateHere: true, runsProton: true,
          nativeOn: ["Linux"], needs: "WinBtrfs, to read it on Windows",
          summary: "Linux at its best: checksums, compression, snapshots." },
      ];
    case "list_games":
      return { problems: [], games: [
        { id: "367520", name: "Hollow Knight", library: "steam", source: "Steam",
          sizeOnDisk: 9_106_886_656, hasCover: true,
          executable: "steam://rungameid/367520", canCopy: true },
        { id: "1145360", name: "Hades", library: "steam", source: "Steam",
          sizeOnDisk: 15_204_593_664, hasCover: false,
          executable: "steam://rungameid/1145360", canCopy: true },
        { id: "b7f3-tunic", name: "Tunic", library: "playnite", source: "GOG",
          sizeOnDisk: 3_221_225_472, hasCover: false,
          executable: "playnite://playnite/start/b7f3-tunic", canCopy: true },
        { id: "c8a1-outer", name: "Outer Wilds", library: "playnite", source: "Epic",
          sizeOnDisk: 0, hasCover: false,
          executable: "playnite://playnite/start/c8a1-outer", canCopy: false },
        { id: "d9b2-halo", name: "Halo Infinite", library: "playnite", source: "Xbox",
          sizeOnDisk: 0, hasCover: false,
          executable: "playnite://playnite/start/d9b2-halo", canCopy: false },
        { id: "413150", name: "Stardew Valley", library: "steam", source: "Steam",
          sizeOnDisk: 1_006_632_960, hasCover: false,
          executable: "steam://rungameid/413150", canCopy: true },
      ] };
    case "get_settings":
      // The preview mirrors a fresh install: offline until switched on.
      return { steamgriddbEnabled: false, steamgriddbApiKey: "" };
    case "set_settings":
      return args.settings;
    case "suggest_collection_name": {
      const words = args.titles.map((t) => t.trim().split(/\s+/));
      const shared = [];
      for (let i = 0; i < words[0].length; i += 1) {
        const word = words[0][i];
        if (!words.every((w) => w[i]?.toLowerCase() === word.toLowerCase())) break;
        shared.push(word);
      }
      return shared.length >= 2
        ? `${shared.join(" ")} Collection`
        : `${args.titles[0]} and ${args.titles.length - 1} more`;
    }
    case "read_cartridge_for_edit":
      return {
        drivePath: args.drivePath,
        title: "God of War Collection",
        cover: "src/demo/gow-collection.jpg",
        coverPath: "D:\\collection.jpg",
        isBundle: true,
        holdsGame: true,
        games: [
          { title: "God of War (2018)", executable: "steam://rungameid/1593500", cover: "", coverPath: "" },
          { title: "God of War Ragnarök", executable: "steam://rungameid/2322010", cover: "", coverPath: "" },
        ],
      };
    case "update_cartridge":
      return { confPath: `${args.request.drivePath}/cartridge.conf`, warnings: [] };
    case "memcard_games":
      return args.drivePath.endsWith("CINDER")
        ? [{ title: "Stardew Valley", executable: "steam://rungameid/413150",
             saveWindows: "{appdata}/StardewValley/Saves", saveLinux: "{appdata}/StardewValley/Saves",
             icon: "src/demo/cover.jpg", iconSource: "/run/media/harry/CINDER/.gamepak/card_0.png" }]
        : [];
    case "memory_card":
      return { title: "Harry's Memory Card", cardOnly: true, blocks: [] };
    case "lookup_save_location":
      await new Promise((r) => setTimeout(r, 400));
      // A few that Ludusavi's list knows, in the placeholders
      // core/src/ludusavi.rs turns its paths into; anything else it does not.
      return {
        "Hades": { windows: "{documents}/Saved Games/Hades", linux: null },
        "Hollow Knight": { windows: "{home}/AppData/LocalLow/Team Cherry/Hollow Knight",
                           linux: "{appdata}/unity3d/Team Cherry/Hollow Knight" },
        "Tunic": { windows: "{home}/AppData/LocalLow/Andrew Shouldice/Secret Legend/SAVES",
                   linux: "{appdata}/unity3d/Andrew Shouldice/Secret Legend/SAVES" },
      }[args.title] ?? null;
    case "pick_save_folder":
      return { path: "/home/harry/.local/share/Tunic", template: "{localappdata}/Tunic" };
    case "create_memory_card":
      return args.request.games.filter((g) => !(g.save || g.saveWindows || g.saveLinux))
        .map((g) => `${g.title} has no save folder, so it was left off.`);
    case "list_created":
      return demoShelf;
    case "forget_created":
      demoShelf = demoShelf.filter((entry) => entry.id !== args.id);
      return null;
    case "host_platform":
      return new URLSearchParams(location.search).get("platform") || "linux";
    case "frontends":
      return [
        { id: "launcher", name: "PC GamePak launcher", on: true, implemented: true, installed: true },
        { id: "decky", name: "Steam Deck (Decky)", on: false, implemented: true, installed: false },
        { id: "playnite", name: "Playnite", on: false, implemented: false, installed: false },
      ];
    case "set_frontend":
      return true;
    case "tuning_plan":
      return [
        "Add-MpPreference -ExclusionPath 'D:\\'",
        "Get-CimInstance -ClassName Win32_Volume -Filter \"DriveLetter='D:'\" | Set-CimInstance -Property @{IndexingEnabled=$false}",
      ];
    case "apply_tuning":
      return args.applying
        ? [
            "Defender scans D:\\ again — left excluded by a cartridge that has since moved letter.",
            "Excluded the cartridge from Defender scanning.",
            "Search indexing switched off for the cartridge.",
          ]
        : ["Defender scans the cartridge again.", "Search indexing switched back on."];
    case "pick_cover_image":
      return { path: "/home/harry/Pictures/collection.jpg", preview: "src/demo/gow-collection.jpg" };
    case "game_cover":
      return args.id === "367520" ? "src/demo/cover.jpg" : "";
    case "sgdb_last_used_artwork":
      return null;
    case "sgdb_search_games":
      return [{ id: 1, name: args.query || "Hollow Knight" }];
    case "sgdb_get_artwork":
      return [
        { id: 7001, url: "https://cdn.steamgriddb.com/grid/demo-7001.jpg",
          thumb: "src/demo/cover.jpg", width: 600, height: 900 },
        { id: 7002, url: "https://cdn.steamgriddb.com/grid/demo-7002.jpg",
          thumb: "src/demo/gow-collection.jpg", width: 600, height: 900 },
      ];
    case "sgdb_download_artwork":
      await new Promise((r) => setTimeout(r, Number(new URLSearchParams(location.search).get("slow") || 0)));
      return { path: `/tmp/sgdb-cache/${args.cacheKey}-${args.url.split("/").pop()}`,
               dataUri: { grid: "src/demo/cover.jpg", hero: "src/demo/gow-collection.jpg",
                          logo: "src/demo/gow-2.jpg", icon: "src/demo/gow-1.jpg" }[
                 args.cacheKey.split("-").pop()] ?? "src/demo/cover.jpg" };
    case "list_target_drives":
      return [
        { path: "/run/media/harry/CINDER", label: "CINDER",
          totalBytes: 128_035_676_160, freeBytes: 119_014_128_640, hasCartridge: false },
        { path: "/run/media/harry/HOLLOW", label: "HOLLOW",
          totalBytes: 128_035_676_160, freeBytes: 18_253_611_008, hasCartridge: true,
          cartridgeTitle: "God of War Collection" },
      ];
    case "add_gamepak": {
      // As the launcher does: the ID given, or a new one.
      const id = args.id || `gp_${Math.random().toString(16).slice(2, 18).padEnd(16, "0")}`;
      demoGamePaks = demoGamePaks.filter((pak) => pak.id !== id);
      demoGamePaks.push({ id, title: args.title.trim(), triggers: [],
                          action: demoAction(args.kind, args.a.trim(), args.b?.trim()) });
      return id;
    }
    case "registered_gamepaks":
      return structuredClone(demoGamePaks);
    case "last_trigger":
      // The preview "taps" a card two seconds after it starts listening.
      demoTapAt ??= Date.now() + 2000;
      return Date.now() < demoTapAt ? ["gp_3501170818305", 1] : ["gp_nfc-e9105207", demoTapAt];
    case "moonlight_hosts":
      return [{ name: "GAMING-PC", apps: ["Desktop", "Hades"] }];
    case "moonlight_apps":
      return ["Desktop", "Steam Big Picture", "Stardew Valley", "Hades", "Cities: Skylines"];
    case "add_gamepak_trigger":
      demoGamePaks.find((pak) => pak.id === args.id)?.triggers.push(args.trigger);
      return undefined;
    case "remove_gamepak":
      demoGamePaks = demoGamePaks.filter((pak) => pak.id !== args.id);
      return undefined;
    case "remove_gamepak_trigger": {
      const pak = demoGamePaks.find((p) => p.id === args.id);
      if (pak) pak.triggers = pak.triggers.filter((t) => t !== args.trigger);
      return undefined;
    }
    case "executable_choices":
      return [
        { relative: "TUNIC.exe", name: "TUNIC.exe", score: 120 },
        { relative: "bin/launcher.exe", name: "launcher.exe", score: -40 },
        { relative: "unins000.exe", name: "unins000.exe", score: -400 },
      ];
    case "pick_rom_file":
      return { path: "B:\\ROMs\\Chrono Trigger (USA).sfc", name: "Chrono Trigger (USA)",
               sizeBytes: 4_194_304, platform: "SNES" };
    case "pick_game_folder":
      return {
        path: "B:\\Games\\Split_Fiction v1.2",
        name: "Split_Fiction v1.2",
        sizeBytes: 74_088_775_680,
        choices: [
          { relative: "SplitFiction.exe", name: "SplitFiction.exe", score: 120 },
          { relative: "Engine/Binaries/UE4PrereqSetup_x64.exe", name: "UE4PrereqSetup_x64.exe", score: -300 },
          { relative: "unins000.exe", name: "unins000.exe", score: -400 },
        ],
      };
    case "cartridge_health":
      return { link: "10 Gbps", linkMbps: 10_000, transport: "UASP",
               label: args.drivePath.split("/").pop(), filesystem: "exFAT",
               totalBytes: 128_035_676_160, freeBytes: 119_014_128_640,
               usedPercent: 7, warnings: [] };
    case "holds_steam_games":
      return true;
    case "register_with_steam":
      return true;
    case "steam_registration_plan":
      return [
        "Add H:\\ to Steam's library list.",
        "Move F:\\Games\\Steam\\steamapps\\appmanifest_413150.acf aside — it says the game is installed there, and the files are gone.",
      ];
    case "steam_registration":
      return args.drivePath.endsWith("HOLLOW");
    case "unregister_from_steam":
      return true;
    case "eject_drive":
      return true;
    case "format_plan":
      return {
        path: args.drivePath,
        currentLabel: args.drivePath.split("/").pop(),
        device: "/dev/sdb1",
        totalBytes: 128_035_676_160,
        warning: `Everything on ${args.drivePath.split("/").pop()} (128 GB) will be erased.`,
      };
    case "create_cartridge":
      return {
        confPath: `${args.request.drivePath}/cartridge.conf`,
        coverWritten: true,
        autorunWritten: Boolean(args.request.writeIcon),
        icon: null,
        formatted: args.request.formatDrive,
        formattedFilesystem: args.request.formatFilesystem || "exfat",
        gameCopied: args.request.copyGame,
        bytesCopied: args.request.copyGame ? 9_106_886_656 : 0,
        registeredWithSteam: args.request.copyGame && Boolean(args.request.appId),
        gameFolder: args.request.copyGame ? "Games" : null,
        warnings: [],
      };
    default:
      console.log("[preview]", command, args);
      return "";
  }
}

start();
