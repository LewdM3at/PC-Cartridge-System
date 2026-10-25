#!/usr/bin/env node
/**
 * Re-shoot the screenshots in docs/ that the preview can make, and render the
 * promo images in docs/promo/ from them.
 *
 *   npm i -D playwright      # once, anywhere node can find it
 *   node tools/screenshots.mjs              # everything
 *   node tools/screenshots.mjs memcard,promo-memcard   # just these
 *
 * Every shot is the app's own preview (tauri-ui/app, opened in a browser with
 * no Tauri behind it), so it shows exactly what the frontend draws, with the
 * preview's made-up games. Shots are taken at 2x so they stay sharp when a page
 * shows them at their natural size. docs/launcher.png and the other shots with
 * real box art are not made here and are left alone.
 *
 * PLAYWRIGHT_CHROMIUM names a Chromium to use instead of Playwright's own.
 */
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const DOCS = join(ROOT, "docs");

const TYPES = {
  ".html": "text/html", ".js": "text/javascript", ".css": "text/css",
  ".png": "image/png", ".jpg": "image/jpeg", ".svg": "image/svg+xml",
  ".woff2": "font/woff2", ".json": "application/json",
};

/** The repository, served read-only, so the pages load as they would. */
function serve() {
  const server = createServer(async (req, res) => {
    const path = normalize(decodeURIComponent(new URL(req.url, "http://x").pathname));
    try {
      const body = await readFile(join(ROOT, path));
      res.writeHead(200, { "content-type": TYPES[extname(path)] ?? "application/octet-stream" });
      res.end(body);
    } catch {
      res.writeHead(404).end();
    }
  });
  return new Promise((resolve) => server.listen(0, "127.0.0.1", () => resolve(server)));
}

const LAUNCHER = [420, 630];
const WIZARD = [1030, 660]; // the sizes tauri-ui/src-tauri/src/main.rs opens them at

/** Each shot: the window it is taken in, and how to get it there. */
const SHOTS = {
  memcard: [LAUNCHER, async (p) => {
    await p.goto("/tauri-ui/app/index.html?drive=X&state=memcard");
    await p.waitForTimeout(2500);
  }],
  "memcard-sync": [LAUNCHER, async (p) => {
    await p.goto("/tauri-ui/app/index.html?drive=X&state=memcard");
    await p.waitForTimeout(2000);
    await p.click('.mc__block[data-index="1"]'); // God of War: newer on the card
    await p.waitForTimeout(1800); // the ring turns it to the front
  }],
  "memcard-conflict": [LAUNCHER, async (p) => {
    await p.goto("/tauri-ui/app/index.html?drive=X&state=memcard");
    await p.waitForTimeout(2000);
    // Tunic, changed on both. Three on round the ring, where it may be behind
    // the card, so it is stepped to rather than clicked.
    for (let i = 0; i < 3; i += 1) await p.keyboard.press("ArrowRight");
    await p.waitForTimeout(1800);
  }],
  "wizard-card": [WIZARD, async (p) => {
    await p.goto("/tauri-ui/app/create.html");
    await p.waitForTimeout(800);
    // The preview starts offline, as a fresh install does.
    await p.click("#btn-settings");
    await p.check("#set-ludusavi");
    await p.click("#settings-save");
    await p.click("#tab-memcard");
    await p.waitForTimeout(1200);
    for (const name of ["Hades", "Hollow Knight", "Tunic"]) {
      await p.fill("#mcw-search", name);
      await p.click("#mcw-results button >> nth=0");
      await p.waitForTimeout(700); // Ludusavi's answer
    }
    await p.waitForTimeout(1000);
  }],
  "wizard-gamepaks": [WIZARD, async (p) => {
    await p.goto("/tauri-ui/app/create.html");
    await p.waitForTimeout(800);
    await p.click("#tab-actions");
    await p.waitForTimeout(1500);
  }],
  "wizard-gamepak-new": [WIZARD, async (p) => {
    await p.goto("/tauri-ui/app/create.html");
    await p.waitForTimeout(800);
    await p.click("#tab-actions");
    await p.click("#gp-new");
    await p.waitForTimeout(2600); // the preview taps a card after two seconds
    await p.click('.gp-kind[data-kind="remote"]');
    await p.fill("#gp-a", "GAMING-PC");
    await p.click("#gp-ml-find");
    await p.click("#gp-ml-results button >> text=Hades");
    await p.waitForTimeout(300);
  }],
  "wizard-emulated": [WIZARD, async (p) => {
    await p.goto("/tauri-ui/app/create.html");
    await p.waitForTimeout(1000);
    // Adding a game by hand is behind the game picker; open it directly.
    await p.evaluate(() => {
      for (let n = document.getElementById("btn-pick-rom"); n; n = n.parentElement) n.hidden = false;
    });
    await p.click("#btn-pick-rom");
    await p.click("#btn-custom-cover");
    await p.waitForTimeout(400);
    await p.click("#btn-custom-done");
    await p.click("#btn-change-media");
    await p.locator("#pick-dialog li button, #pick-dialog [role=option]").first().click();
    await p.waitForTimeout(1200);
  }],
};

/** Promo images, drawn by docs/promo/promo.html around the shots above. */
const PROMOS = {
  "promo-social": [[1280, 640], "social"], // GitHub's social preview size
  "promo-memcard": [[1200, 675], "memcard"],
  "promo-memcard-wizard": [[1200, 675], "memcard-wizard"],
  "promo-gamepaks": [[1200, 675], "gamepaks"],
  "promo-emulated": [[1200, 675], "emulated"],
};

const only = process.argv[2]?.split(",");
const wanted = (name) => !only || only.includes(name);

const server = await serve();
const base = `http://127.0.0.1:${server.address().port}`;
const browser = await chromium.launch(
  process.env.PLAYWRIGHT_CHROMIUM ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM } : {},
);
let failed = false;
async function shoot(name, [width, height], go, path, options = {}) {
  const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor: 2, baseURL: base });
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  try {
    await go(page);
    await page.mouse.move(0, 0); // no hover left on whatever was clicked last
    await page.screenshot({ path, ...options });
    console.log(errors.length ? `shot ${name}, but the page threw: ${errors.join("; ")}` : `shot ${name}`);
    failed ||= errors.length > 0;
  } catch (error) {
    console.error(`could not shoot ${name}: ${error.message}`);
    failed = true;
  } finally {
    await page.close();
  }
}

for (const [name, [size, go]] of Object.entries(SHOTS)) {
  if (wanted(name)) await shoot(name, size, go, join(DOCS, `${name}.png`));
}
for (const [name, [size, layout]] of Object.entries(PROMOS)) {
  if (!wanted(name)) continue;
  await shoot(name, size, async (p) => {
    await p.goto(`/docs/promo/promo.html?t=${layout}`);
    await p.evaluate(() => document.fonts.ready);
    await p.waitForTimeout(300);
  }, join(DOCS, "promo", `${name}.jpg`), { type: "jpeg", quality: 90 });
}

await browser.close();
server.close();
process.exitCode = failed ? 1 : 0;
