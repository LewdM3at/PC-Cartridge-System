/**
 * Verify every element the frontend reaches for actually exists in its HTML.
 *
 * The UI ships unbundled with no framework, so `getElementById` returning null
 * is not a build error — it is a TypeError the first time that code path runs,
 * which may be the first time someone plugs in a cartridge. This is the cheapest
 * possible guard against a rename in one file and not the other.
 *
 *   node tools/check-dom-ids.mjs
 */
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

// fileURLToPath, not `new URL(...).pathname`: on Windows the latter yields
// "/C:/…", and path.resolve reads the leading slash as relative and prepends
// the drive again, leaving "C:\C:\…".
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/** Which script drives which page. */
const PAGES = [
  { html: "tauri-ui/app/index.html", scripts: ["tauri-ui/app/src/main.js", "tauri-ui/app/src/memcard.js"] },
  { html: "tauri-ui/app/create.html", scripts: ["tauri-ui/app/src/create.js"] },
];

const idsIn = (html) =>
  new Set([...html.matchAll(/\bid\s*=\s*["']([^"']+)["']/g)].map((m) => m[1]));

/**
 * A one-character alias for getElementById, if the file defines one.
 *
 * A file with a hundred lookups tends to shorten them, and a shorthand this
 * check cannot see is worse than no shorthand: the check keeps passing while
 * silently testing nothing. Matching the definition rather than assuming a name
 * keeps that from being a trap.
 */
const aliasIn = (js) => {
  const m = js.match(
    /(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=\s*\(\s*\w+\s*\)\s*=>\s*document\.getElementById\(/,
  );
  return m?.[1] ?? null;
};

const wantedBy = (js) => {
  const wanted = new Map(); // id -> how it was referenced
  for (const m of js.matchAll(/getElementById\(\s*["']([^"']+)["']\s*\)/g)) {
    wanted.set(m[1], `getElementById("${m[1]}")`);
  }
  // Only bare "#id" selectors; anything more complex is not worth guessing at.
  for (const m of js.matchAll(/querySelector(?:All)?\(\s*["']#([A-Za-z0-9_-]+)["']\s*\)/g)) {
    wanted.set(m[1], `querySelector("#${m[1]}")`);
  }

  const alias = aliasIn(js);
  if (alias) {
    const escaped = alias.replace(/[$]/g, "\\$&");
    const calls = new RegExp(`(?<![\\w$.])${escaped}\\(\\s*["']([^"']+)["']\\s*\\)`, "g");
    for (const m of js.matchAll(calls)) {
      wanted.set(m[1], `${alias}("${m[1]}")`);
    }
  }
  return wanted;
};

let failures = 0;

for (const page of PAGES) {
  const html = await fs.readFile(path.join(ROOT, page.html), "utf8");
  const present = idsIn(html);

  for (const script of page.scripts) {
    const js = await fs.readFile(path.join(ROOT, script), "utf8");
    const wanted = wantedBy(js);

    for (const [id, how] of wanted) {
      if (!present.has(id)) {
        console.error(`✗ ${script}: ${how} — no #${id} in ${page.html}`);
        failures += 1;
      }
    }
    // Zero is never right for a script that drives a page, and it is what a
    // broken matcher looks like from the outside.
    if (wanted.size === 0) {
      console.error(`✗ ${script}: no element lookups found — this check is not checking anything`);
      failures += 1;
    }
    console.log(`${script} → ${page.html}: ${wanted.size} ids checked`);
  }
}

if (failures > 0) {
  console.error(`\n${failures} missing element${failures === 1 ? "" : "s"}.`);
  process.exit(1);
}
console.log("all referenced elements exist");
