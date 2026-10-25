/**
 * Verify every place the version is written says the same thing.
 *
 * The version lives in fourteen places across five formats, and every one of
 * them used to be edited by hand. Predictably, they drifted: at the time this
 * was written the crates said 1.0.1, `package-lock.json` said 0.1.0, and the AUR
 * and WinGet manifests said 1.0.0 — so two of the three channels people install
 * from were pointing at a release older than the code, and nothing anywhere
 * noticed.
 *
 * `core/Cargo.toml` is the source of truth, because it is the crate the other two
 * depend on and the one a human edits first.
 *
 *   node tools/check-versions.mjs            every site agrees, or exit 1
 *   node tools/check-versions.mjs --set 1.1.0  move them all at once
 *   node tools/check-versions.mjs --release   also: no checksum placeholders
 *
 * The `--release` mode exists because two sites carry a **checksum of a release
 * artefact**, which cannot be known until the release exists. Bumping the version
 * replaces those with a placeholder, which is the correct state between the bump
 * and the tag — and is not a state to publish an AUR or WinGet manifest in. CI
 * runs the default mode and tolerates them; `docs/PUBLISHING.md` says to run
 * `--release` before submitting a manifest anywhere.
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

// fileURLToPath, not `new URL(...).pathname`: see check-dom-ids.mjs for the
// Windows drive-letter reason.
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/** What a checksum reads as between a version bump and the release it names. */
const PENDING = "SHA256-PENDING-RELEASE";

const read = (rel) => fs.readFileSync(path.join(ROOT, rel), "utf8");
const write = (rel, text) => fs.writeFileSync(path.join(ROOT, rel), text);

/**
 * One place a version is written.
 *
 * `find` returns every version string in the file, so a file naming it twice is
 * checked twice rather than half-checked. `swap` does the replacement, taking the
 * old and new version so a site that embeds the version in a URL can rewrite the
 * whole line rather than guessing at its shape.
 */
const site = (file, find, swap) => ({ file, find, swap });

/** `version = "x"` in a Cargo.toml's own [package] section — the first one. */
const cargoToml = (file) =>
  site(
    file,
    (text) => [text.match(/^version = "([^"]+)"/m)?.[1]].filter(Boolean),
    (text, _old, next) => text.replace(/^version = "[^"]+"/m, `version = "${next}"`),
  );

/**
 * A named package's version inside a Cargo.lock.
 *
 * Cargo rewrites this on the next build whatever it says, so a stale lockfile is
 * a dirty working tree waiting to happen rather than a broken build — which is
 * exactly the kind of thing nobody notices until a release is being cut.
 */
const cargoLock = (file, pkg) => {
  const pattern = new RegExp(`(name = "${pkg}"\\r?\\nversion = ")([^"]+)(")`);
  return site(
    file,
    (text) => [text.match(pattern)?.[2]].filter(Boolean),
    (text, _old, next) => text.replace(pattern, `$1${next}$3`),
  );
};

/** A top-level `"version": "x"` in a JSON file, and only the top-level one. */
const jsonVersion = (file) =>
  site(
    file,
    (text) => [text.match(/^ {2}"version": "([^"]+)"/m)?.[1]].filter(Boolean),
    (text, _old, next) => text.replace(/^( {2}"version": ")[^"]+(")/m, `$1${next}$2`),
  );

/**
 * Every occurrence of the old version in a file, replaced literally.
 *
 * `optional`, because these are versions embedded in URLs and filenames and only
 * some of a manifest set carries them — WinGet's installer file names the
 * download twice and its locale file not at all. A site that legitimately has
 * none must not read as a site that lost one.
 */
const everyMention = (file, pattern) => ({
  ...site(
    file,
    (text) => [...text.matchAll(pattern)].map((match) => match[1]),
    // Each match's *own* version is replaced, not the version the canonical file
    // happened to hold. The difference matters precisely when it matters: these
    // sites had drifted a release behind, so a `--set` that only rewrote the
    // previous canonical number would have walked straight past them.
    (text, _old, next) => {
      let out = text;
      for (const found of new Set([...text.matchAll(pattern)].map((m) => m[1]))) {
        if (found !== next) out = out.split(found).join(next);
      }
      return out;
    },
  ),
  optional: true,
});

const SITES = [
  cargoToml("core/Cargo.toml"),
  cargoToml("watcher/Cargo.toml"),
  cargoToml("tauri-ui/src-tauri/Cargo.toml"),
  cargoLock("core/Cargo.lock", "gamepak-core"),
  cargoLock("watcher/Cargo.lock", "pc-gamepak-watcher"),
  cargoLock("tauri-ui/src-tauri/Cargo.lock", "pc-gamepak"),
  cargoLock("tauri-ui/src-tauri/Cargo.lock", "gamepak-core"),
  jsonVersion("tauri-ui/package.json"),
  jsonVersion("tauri-ui/package-lock.json"),
  // package-lock names it twice: once for the tree and once for the root
  // package. npm rewrites whichever one is wrong, silently.
  site(
    "tauri-ui/package-lock.json",
    (text) => [text.match(/"": \{\r?\n {6}"name": "[^"]+",\r?\n {6}"version": "([^"]+)"/)?.[1]].filter(Boolean),
    (text, _old, next) =>
      text.replace(
        /("": \{\r?\n {6}"name": "[^"]+",\r?\n {6}"version": ")[^"]+(")/,
        `$1${next}$2`,
      ),
  ),
  jsonVersion("tauri-ui/src-tauri/tauri.conf.json"),
  site(
    "packaging/aur/pc-gamepak/PKGBUILD",
    (text) => [text.match(/^pkgver=(.+)$/m)?.[1]].filter(Boolean),
    (text, _old, next) => text.replace(/^pkgver=.+$/m, `pkgver=${next}`),
  ),
  site(
    "packaging/aur/pc-gamepak/.SRCINFO",
    (text) => [text.match(/^\tpkgver = (.+)$/m)?.[1]].filter(Boolean),
    (text, _old, next) => text.replace(/^\tpkgver = .+$/m, `\tpkgver = ${next}`),
  ),
  // The tarball name and the tag it is fetched from, both carrying the version.
  everyMention("packaging/aur/pc-gamepak/.SRCINFO", /pc-gamepak-([0-9][^.]*\.[^.]*\.[^.]*)\.tar\.gz/g),
  site(
    "packaging/flatpak/io.github.HarryBMa.PCGamePak.yml",
    (text) => [text.match(/^\s*tag: v(.+)$/m)?.[1]].filter(Boolean),
    (text, _old, next) => text.replace(/^(\s*tag: v).+$/m, `$1${next}`),
  ),
  // The newest <release> entry. Checked here, but never edited here — see
  // `addFlatpakRelease`: rewriting the version on the existing entry would put
  // the new number above the previous release's description, quietly turning
  // appstream's history into a lie.
  site(
    "packaging/flatpak/io.github.HarryBMa.PCGamePak.metainfo.xml",
    (text) => [text.match(/<release version="([^"]+)"/)?.[1]].filter(Boolean),
    (text) => text,
  ),
];

/** The file appstream reads, whose <releases> block is a history. */
const METAINFO = "packaging/flatpak/io.github.HarryBMa.PCGamePak.metainfo.xml";

/**
 * Put a new release at the top of the metainfo's history.
 *
 * Prepended, with the older entries left exactly as they are. The description is
 * a placeholder on purpose: appstream shows it in software centres, nobody can
 * write it from a version number, and a visible `TODO` is a better outcome than
 * inheriting the previous release's notes under a new number.
 */
function addFlatpakRelease(version) {
  const text = read(METAINFO);
  if (text.includes(`<release version="${version}"`)) return false;
  const today = new Date().toISOString().slice(0, 10);
  const entry =
    `  <releases>\n` +
    `    <release version="${version}" date="${today}">\n` +
    `      <description>\n` +
    `        <p>TODO: what changed, in a sentence someone browsing a software\n` +
    `        centre would find useful. See CHANGELOG.md.</p>\n` +
    `      </description>\n` +
    `    </release>\n`;
  write(METAINFO, text.replace(/ {2}<releases>\n/, entry));
  return true;
}

/** The date the metainfo gives this version, which software centres show. */
function metainfoDate(version) {
  const escaped = version.replace(/\./g, "\\.");
  return (
    read(METAINFO).match(new RegExp(`<release version="${escaped}" date="([^"]+)"`))?.[1] ?? null
  );
}

/**
 * The changelog heading for a version, and the date in it if there is one.
 *
 * The release workflow cuts its notes out of the section under this heading and
 * falls back to commit subjects when there is none — quietly, which is why it is
 * worth checking here rather than reading the published release to find out.
 */
function changelogHeading(version) {
  const escaped = version.replace(/\./g, "\\.");
  const line = read("CHANGELOG.md").match(new RegExp(`^## ${escaped}(?: .*)?$`, "m"))?.[0];
  return line ? { line: line.trim(), date: line.match(/\d{4}-\d{2}-\d{2}/)?.[0] ?? null } : null;
}

/** The WinGet manifests, whose *directory* is named for the version. */
function wingetDir() {
  const parent = path.join(ROOT, "packaging/winget");
  return fs
    .readdirSync(parent, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort()
    .pop();
}

function wingetSites(version) {
  const dir = `packaging/winget/${version}`;
  return fs
    .readdirSync(path.join(ROOT, dir))
    .filter((name) => name.endsWith(".yaml"))
    .flatMap((name) => [
      site(
        `${dir}/${name}`,
        (text) => [text.match(/^PackageVersion: (.+)$/m)?.[1]].filter(Boolean),
        (text, _old, next) => text.replace(/^PackageVersion: .+$/m, `PackageVersion: ${next}`),
      ),
      everyMention(`${dir}/${name}`, /\/download\/v([^/]+)\//g),
      everyMention(`${dir}/${name}`, /pc-gamepak-([0-9][^-]*)-windows/g),
    ]);
}

/** Where a checksum of a release artefact is written down. */
const CHECKSUMS = [
  ["packaging/aur/pc-gamepak/PKGBUILD", /sha256sums=\('([^']+)'\)/],
  ["packaging/aur/pc-gamepak/.SRCINFO", /^\tsha256sums = (.+)$/m],
];

function checksumSites(version) {
  const dir = `packaging/winget/${version}`;
  const installer = fs
    .readdirSync(path.join(ROOT, dir))
    .find((name) => name.includes("installer"));
  return installer
    ? [...CHECKSUMS, [`${dir}/${installer}`, /InstallerSha256: (.+)/]]
    : CHECKSUMS;
}

// --------------------------------------------------------------------------

const args = process.argv.slice(2);
const setIndex = args.indexOf("--set");
const target = setIndex >= 0 ? args[setIndex + 1] : null;
const forRelease = args.includes("--release");

if (setIndex >= 0 && !/^\d+\.\d+\.\d+$/.test(target ?? "")) {
  console.error("usage: check-versions.mjs --set <major.minor.patch>");
  process.exit(2);
}

const current = read("core/Cargo.toml").match(/^version = "([^"]+)"/m)?.[1];
if (!current) {
  console.error("core/Cargo.toml has no version; nothing to compare against");
  process.exit(2);
}

if (target) {
  // The WinGet directory is named for the version, so it is moved rather than
  // edited. Done first, so the sites below are read from their new home.
  const oldDir = wingetDir();
  if (oldDir && oldDir !== target) {
    const from = path.join(ROOT, "packaging/winget", oldDir);
    const to = path.join(ROOT, "packaging/winget", target);
    fs.mkdirSync(to, { recursive: true });
    for (const name of fs.readdirSync(from)) {
      fs.renameSync(path.join(from, name), path.join(to, name));
    }
    fs.rmdirSync(from);
    console.log(`moved packaging/winget/${oldDir} -> packaging/winget/${target}`);
  }

  let touched = 0;
  if (addFlatpakRelease(target)) {
    touched += 1;
    console.log(`added a <release> entry for ${target} to the metainfo`);
  }
  for (const one of [...SITES, ...wingetSites(target)]) {
    const before = read(one.file);
    const after = one.swap(before, current, target);
    if (after !== before) {
      write(one.file, after);
      touched += 1;
    }
  }

  // A checksum names an artefact of a specific release, so it cannot survive the
  // version changing under it. Blanked rather than left lying: a stale checksum
  // is worse than an obviously missing one, because it looks like an answer.
  for (const [file, pattern] of checksumSites(target)) {
    const before = read(file);
    const found = before.match(pattern)?.[1];
    if (found && found !== PENDING) {
      write(file, before.replace(found, PENDING));
      touched += 1;
    }
  }

  console.log(`${current} -> ${target} across ${touched} places`);
  console.log(
    `\nStill to do, and they cannot be done from here:\n` +
      `  1. write the CHANGELOG entry, and the metainfo description\n` +
      `  2. tag v${target} and let the release workflow build it\n` +
      `  3. put the real checksums where ${PENDING} is now\n` +
      `  4. re-run with --release to confirm none are left`,
  );
  process.exit(0);
}

// ---- checking -------------------------------------------------------------

let bad = 0;
let pending = 0;
const seen = new Map();

for (const one of [...SITES, ...wingetSites(wingetDir())]) {
  const found = one.find(read(one.file));
  if (found.length === 0) {
    if (one.optional) continue;
    console.error(`${one.file}: no version found where one was expected`);
    bad += 1;
    continue;
  }
  for (const version of found) {
    const key = `${one.file} — ${version}`;
    if (!seen.has(key)) seen.set(key, version);
    if (version !== current) {
      console.error(`${one.file}: says ${version}, core/Cargo.toml says ${current}`);
      bad += 1;
    }
  }
}

const wingetName = wingetDir();
if (wingetName !== current) {
  console.error(`packaging/winget/${wingetName}: directory is not named ${current}`);
  bad += 1;
}

for (const [file, pattern] of checksumSites(wingetName)) {
  const found = read(file).match(pattern)?.[1];
  if (found === PENDING) {
    pending += 1;
    console.log(`${file}: checksum still ${PENDING}`);
  }
}

// The changelog and the metainfo both publish a date for the same release, and
// nothing until now compared them. Only a disagreement is drift; a section that
// is still `unreleased`, or missing, is an unfinished step, which is `--release`
// business rather than a reason to fail every check on the way there.
const unreleased = [];
const heading = changelogHeading(current);
const dated = metainfoDate(current);
if (!heading) {
  unreleased.push(
    `CHANGELOG.md has no "## ${current}" section, so the release notes would be built from commit subjects`,
  );
} else if (!heading.date) {
  unreleased.push(`CHANGELOG.md still says "${heading.line}" — put the release date there`);
} else if (dated && heading.date !== dated) {
  console.error(
    `CHANGELOG.md dates ${current} ${heading.date}, the metainfo dates it ${dated}. One release, one date.`,
  );
  bad += 1;
}

if (bad > 0) {
  console.error(
    `\n${bad} place${bad === 1 ? " disagrees" : "s disagree"}. ` +
      `\`--set ${current}\` settles the version sites; a date has to be fixed by hand.`,
  );
  process.exit(1);
}

console.log(`every version site says ${current}`);

let stopping = false;
if (pending > 0) {
  const message = `${pending} checksum${pending === 1 ? "" : "s"} awaiting the release artefacts`;
  if (forRelease) {
    console.error(`${message} — fill them in before publishing a manifest`);
    stopping = true;
  } else {
    console.log(`${message} (fine until a manifest is submitted)`);
  }
}

for (const message of unreleased) {
  if (forRelease) {
    console.error(message);
    stopping = true;
  } else {
    console.log(`${message} (fine until the tag)`);
  }
}

if (stopping) process.exit(1);
