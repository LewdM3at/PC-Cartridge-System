#!/usr/bin/env node
/**
 * Record the animated demos in docs/ from the preview, as GIFs.
 *
 *   npm i -D playwright      # once, as for tools/screenshots.mjs
 *   node tools/demo-gifs.mjs                 # every demo
 *   node tools/demo-gifs.mjs memcard-demo    # just this one
 *
 * Each demo is driven like a person would: a pointer glides to a control and
 * clicks it. Playwright records the page as video (it draws no pointer, so one
 * is added to the page), and ffmpeg turns the video into a GIF with a palette
 * made for it. Needs ffmpeg on PATH, or FFMPEG naming it.
 *
 * Video is recorded at the page's own size whatever the pixel density, so the
 * GIFs are 1x; a bigger recording only pads the picture.
 *
 * PLAYWRIGHT_CHROMIUM names a Chromium to use instead of Playwright's own.
 */
import { createServer } from "node:http";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { extname, join, normalize } from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const FFMPEG = process.env.FFMPEG ?? "ffmpeg";

const TYPES = {
  ".html": "text/html", ".js": "text/javascript", ".css": "text/css",
  ".png": "image/png", ".jpg": "image/jpeg", ".svg": "image/svg+xml",
  ".woff2": "font/woff2", ".json": "application/json",
};

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

/** A pointer the video can see: a ring that follows the mouse and pulses on a click. */
const POINTER = () => {
  addEventListener("DOMContentLoaded", () => {
    const ring = document.createElement("div");
    ring.style.cssText = `position:fixed;z-index:2147483647;left:0;top:0;width:22px;height:22px;
      margin:-11px 0 0 -11px;border-radius:50%;pointer-events:none;
      background:oklch(1 0 0 / .22);border:2px solid oklch(1 0 0 / .9);
      box-shadow:0 2px 8px oklch(0 0 0 / .6);transition:transform .12s ease;
      transform:translate(-100px,-100px)`;
    document.body.append(ring);
    let x = -100, y = -100, down = false;
    const draw = () => (ring.style.transform = `translate(${x}px,${y}px) scale(${down ? 0.7 : 1})`);
    addEventListener("mousemove", (e) => { x = e.clientX; y = e.clientY; draw(); }, true);
    addEventListener("mousedown", () => { down = true; draw(); }, true);
    addEventListener("mouseup", () => { down = false; draw(); }, true);
  });
};

/** Move the pointer to the middle of `selector` the way a hand would, then click. */
async function glideClick(page, selector, { pause = 250 } = {}) {
  const target = page.locator(selector).first();
  // Off screen, or off the edge of a scrolling shelf, it is scrolled to first,
  // as a person would; aiming at where it would be would click nothing.
  await target.scrollIntoViewIfNeeded();
  await page.waitForTimeout(300);
  const box = await target.boundingBox();
  if (!box) throw new Error(`nothing to click at ${selector}`);
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2, { steps: 22 });
  await page.waitForTimeout(pause);
  await page.mouse.down();
  await page.waitForTimeout(90);
  await page.mouse.up();
}

/** Type like a person, so a field fills in front of the viewer. */
async function typeInto(page, selector, text) {
  await glideClick(page, selector);
  await page.keyboard.type(text, { delay: 70 });
}

const DEMOS = {
  // A combo cartridge's memory card: the card slots in and its saves fan out,
  // a save newer on the card is copied home, the ring turns to one changed on
  // both, and back to the cartridge.
  "memcard-demo": {
    viewport: [420, 630],
    width: 420, // the GIF's width
    async setup(page) {
      await page.goto("/tauri-ui/app/index.html?drive=X&combo");
      await page.waitForTimeout(2200);
      await page.mouse.move(210, 420);
    },
    async run(page) {
      await page.waitForTimeout(700);
      await glideClick(page, "#btn-memcard");
      await page.waitForTimeout(1700); // in it goes, and out come the saves
      // The card and the chosen case lean after the pointer.
      await page.mouse.move(120, 330, { steps: 26 });
      await page.mouse.move(310, 300, { steps: 30 });
      await page.waitForTimeout(400);
      await glideClick(page, '.mc__block[data-index="1"]'); // God of War
      await page.waitForTimeout(1500);
      await glideClick(page, "#mc-to-pc");
      await page.waitForTimeout(2200);
      await glideClick(page, "#mc-next");
      await page.waitForTimeout(900);
      await glideClick(page, "#mc-next"); // Tunic: changed on both
      await page.waitForTimeout(2300);
      await glideClick(page, "#mc-close");
      await page.waitForTimeout(1200);
    },
  },
  // A card tapped at New GamePak becomes a game on another PC in four clicks.
  "gamepak-demo": {
    viewport: [1030, 660],
    width: 824,
    async setup(page) {
      await page.goto("/tauri-ui/app/create.html");
      await page.waitForTimeout(800);
      await page.click("#tab-actions");
      await page.waitForTimeout(800);
      await page.mouse.move(700, 420);
    },
    async run(page) {
      await page.waitForTimeout(700);
      await glideClick(page, "#gp-new");
      await page.waitForTimeout(2800); // the preview taps a card two seconds in
      await glideClick(page, '.gp-kind[data-kind="remote"]');
      await page.waitForTimeout(500);
      await typeInto(page, "#gp-a", "GAMING-PC");
      await page.waitForTimeout(300);
      await glideClick(page, "#gp-ml-find");
      await page.waitForTimeout(900);
      await glideClick(page, "#gp-ml-results button >> text=Hades");
      await page.waitForTimeout(900);
      await glideClick(page, "#gp-save");
      await page.waitForTimeout(2600);
    },
  },
};

/** Video to GIF, with a palette made from this video so the colours hold. */
function toGif(video, gif, startSeconds, width) {
  const filters = `fps=15,scale=${width}:-1:flags=lanczos,split[a][b];` +
    "[a]palettegen=stats_mode=diff:max_colors=192[p];" +
    "[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle";
  execFileSync(FFMPEG, [
    "-v", "error", "-y", "-ss", startSeconds.toFixed(2), "-i", video,
    "-filter_complex", filters, "-loop", "0", gif,
  ]);
}

const only = process.argv[2]?.split(",");
const server = await serve();
const base = `http://127.0.0.1:${server.address().port}`;
const browser = await chromium.launch(
  process.env.PLAYWRIGHT_CHROMIUM ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM } : {},
);
const work = await mkdtemp(join(tmpdir(), "pc-gamepak-gifs-"));
let failed = false;

for (const [name, demo] of Object.entries(DEMOS)) {
  if (only && !only.includes(name)) continue;
  const [width, height] = demo.viewport;
  const context = await browser.newContext({
    viewport: { width, height },
    baseURL: base,
    recordVideo: { dir: work, size: { width, height } },
  });
  await context.addInitScript(POINTER);
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  const opened = Date.now();
  try {
    await demo.setup(page);
    // Everything before here is loading; the GIF starts where the demo does.
    const start = (Date.now() - opened) / 1000;
    await demo.run(page);
    await context.close(); // finishes the video file
    toGif(await page.video().path(), join(ROOT, "docs", `${name}.gif`), start, demo.width);
    console.log(errors.length ? `recorded ${name}, but the page threw: ${errors.join("; ")}` : `recorded ${name}`);
    failed ||= errors.length > 0;
  } catch (error) {
    console.error(`could not record ${name}: ${error.message}`);
    failed = true;
    await context.close().catch(() => {});
  }
}

await browser.close();
server.close();
await rm(work, { recursive: true, force: true });
process.exitCode = failed ? 1 : 0;
