/**
 * Build every icon from the PC-GAMEPAK artwork, drawn as vectors at each size.
 *
 * Two icons, so the two windows of one executable can be told apart in the
 * taskbar:
 *
 *   the app     the six colour bars of the PC-GAMEPAK mark on a navy tile: the
 *               launcher's executable, its taskbar button, and the watcher's
 *               notification-area icon (the watcher embeds icon.ico)
 *   the wizard  a cartridge, with the six bars on its label
 *
 * No letters: at 16 px a word is a smudge, and the mark says PC-GAMEPAK
 * without one. Both are drawn here from the mark's own geometry (its bars'
 * parallelograms) rather than shrunk from a picture, and the smallest sizes
 * get their own proportions: the tile fills its square, the bars widen to
 * close their gaps, and the cartridge's label grows, so a few pixels still
 * read on a light or a dark taskbar.
 *
 *   node tools/make-icons.mjs
 *
 * Needs `playwright` or `playwright-core` and a Chromium-family browser (Edge
 * on Windows): the browser draws the SVG, so nothing else is installed.
 */
import path from "node:path";
import fs from "node:fs/promises";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const ICONS = path.join(ROOT, "tauri-ui/src-tauri/icons");

// The sizes Tauri's bundler expects, and the ones that go into the .ico.
const PNGS = [
  ["32x32.png", 32],
  ["128x128.png", 128],
  ["128x128@2x.png", 256],
  ["icon.png", 512],
  ["Square150x150Logo.png", 150],
  ["Square44x44Logo.png", 44],
];
// Largest first: Tauri builds its default window icon from the .ico's first
// entry alone, and Windows picks the best size whatever the order.
const ICO_SIZES = [256, 128, 96, 64, 48, 40, 32, 24, 20, 16];
// The wizard's window icon. Large, because Windows is asked for it at the
// window's DPI and shrinks it: 48 px for a taskbar at 150%.
const WIZARD_SIZE = 256;
// This size and below is "small": drawn with the bolder proportions.
const SMALL = 24;

const STRIPES = ["#e8323c", "#f7901e", "#fbcd1f", "#21b35b", "#1e88e5", "#8e4bc9"];
const CREAM = "#f4f1ea";

/** The app: the mark's six bars on a navy tile. Drawn on a 256 square. */
function appSvg(size) {
  const small = size <= SMALL;
  const inset = small ? 0 : 8;
  const side = 256 - inset * 2;
  const radius = small ? 48 : 52;
  // The mark's bar: 22.8 wide, 120 tall, leaning 50.4 over its height, one
  // every 28.8. On a square tile the bars stand taller than the mark's, to
  // fill it; small sizes stand them taller still and more upright, and close
  // the gaps, so each bar is a solid pixel or two.
  const pitch = 28.8;
  const bar = small ? pitch : 22.8;
  const lean = small ? 30 : 50.4;
  const tall = small ? 175 : 150;
  const span = pitch * 5 + bar + lean;
  const scale = (side * (small ? 0.86 : 0.76)) / span;
  const x0 = 128 - (span * scale) / 2;
  const h = tall * scale;
  const y0 = 128 - h / 2;
  const bars = STRIPES.map((colour, i) => {
    const x = x0 + (i * pitch + lean) * scale;
    const w = bar * scale;
    const l = lean * scale;
    return `<path d="M${x} ${y0}h${w}l${-l} ${h}h${-w}z" fill="${colour}"/>`;
  }).join("");
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 256 256">
<defs><linearGradient id="t" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#2a3584"/><stop offset="1" stop-color="#0a0e33"/></linearGradient></defs>
<rect x="${inset}" y="${inset}" width="${side}" height="${side}" rx="${radius}" fill="url(#t)"/>
${bars}
<rect x="${inset + 1.5}" y="${inset + 1.5}" width="${side - 3}" height="${side - 3}" rx="${radius - 1.5}" fill="none" stroke="#fff" stroke-opacity="${small ? 0.3 : 0.18}" stroke-width="3"/>
</svg>`;
}

/**
 * The wizard: a cartridge, the thing it makes. A navy shell with its grip
 * notches across the top and chamfered corners at the foot, and a cream label
 * carrying the mark's six bars, cut to the label so none run over its edge.
 * Drawn on a 256 square; small sizes get a bigger label, fewer notches and
 * bolder bars.
 */
function wizardSvg(size) {
  const small = size <= SMALL;
  const shell = small
    ? "M28 10h200a14 14 0 0 1 14 14v200l-22 22H36l-22-22V24a14 14 0 0 1 14-14z"
    : "M48 20h160a12 12 0 0 1 12 12v188l-18 18H54l-18-18V32a12 12 0 0 1 12-12z";
  const notches = small ? 3 : 6;
  const notchW = small ? 22 : 10;
  const notchGap = small ? 48 : 20;
  const notchX = 128 - ((notches - 1) * notchGap + notchW) / 2;
  const grip = Array.from({ length: notches }, (_, i) =>
    `<rect x="${notchX + i * notchGap}" y="${small ? 20 : 30}" width="${notchW}" height="${small ? 30 : 26}" rx="${small ? 6 : 3}" fill="#fff" fill-opacity="${small ? 0.28 : 0.18}"/>`,
  ).join("");
  const label = small ? { x: 34, y: 66, w: 188, h: 150, r: 16 } : { x: 58, y: 72, w: 140, h: 130, r: 12 };
  // The bars, as on the app's tile, filling the label and clipped to it.
  const pitch = 28.8;
  const bar = small ? pitch : 22.8;
  const lean = small ? 30 : 40;
  const tall = 150;
  const span = pitch * 5 + bar + lean;
  const scale = (label.w * (small ? 0.92 : 0.84)) / span;
  const h = Math.min(tall * scale, label.h * (small ? 0.86 : 0.7));
  const x0 = label.x + (label.w - span * scale) / 2;
  const y0 = label.y + (label.h - h) / 2;
  const bars = STRIPES.map((colour, i) => {
    const x = x0 + (i * pitch + lean) * scale;
    return `<path d="M${x} ${y0}h${bar * scale}l${-lean * scale} ${h}h${-bar * scale}z" fill="${colour}"/>`;
  }).join("");
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 256 256">
<defs>
<linearGradient id="t" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#2a3584"/><stop offset="1" stop-color="#0a0e33"/></linearGradient>
<clipPath id="label"><rect x="${label.x}" y="${label.y}" width="${label.w}" height="${label.h}" rx="${label.r}"/></clipPath>
</defs>
<path d="${shell}" fill="url(#t)"/>
${grip}
<rect x="${label.x}" y="${label.y}" width="${label.w}" height="${label.h}" rx="${label.r}" fill="${CREAM}"/>
<g clip-path="url(#label)">${bars}</g>
<path d="${shell}" fill="none" stroke="#fff" stroke-opacity="${small ? 0.3 : 0.18}" stroke-width="3"/>
</svg>`;
}

let chromium;
try {
  ({ chromium } = await import("playwright"));
} catch {
  ({ chromium } = await import("playwright-core"));
}
const browser = await chromium.launch(process.platform === "win32" ? { channel: "msedge" } : {});
const tab = await browser.newPage();
await tab.setContent("<!doctype html><body></body>");

/** Draw an SVG at exactly `size` pixels, as PNG bytes. */
async function png(svg, size) {
  const base64 = await tab.evaluate(
    async ({ svg, size }) => {
      const img = new Image();
      img.src = `data:image/svg+xml;base64,${btoa(svg)}`;
      await img.decode();
      const canvas = Object.assign(document.createElement("canvas"), { width: size, height: size });
      canvas.getContext("2d").drawImage(img, 0, 0, size, size);
      return canvas.toDataURL("image/png").split(",")[1];
    },
    { svg, size },
  );
  return Buffer.from(base64, "base64");
}

// Multi-resolution .ico. Each entry stores a PNG verbatim, which Windows has
// accepted since Vista and keeps the file small at 256px.
function ico(entries) {
  const header = Buffer.alloc(6 + entries.length * 16);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2); // type: icon
  header.writeUInt16LE(entries.length, 4);
  let offset = header.length;
  entries.forEach((entry, i) => {
    const at = 6 + i * 16;
    header.writeUInt8(entry.size === 256 ? 0 : entry.size, at); // 256 encodes as 0
    header.writeUInt8(entry.size === 256 ? 0 : entry.size, at + 1);
    header.writeUInt16LE(1, at + 4); // colour planes
    header.writeUInt16LE(32, at + 6); // bits per pixel
    header.writeUInt32LE(entry.data.length, at + 8);
    header.writeUInt32LE(offset, at + 12);
    offset += entry.data.length;
  });
  return Buffer.concat([header, ...entries.map((e) => e.data)]);
}

for (const [name, size] of PNGS) {
  await fs.writeFile(path.join(ICONS, name), await png(appSvg(size), size));
  console.log(`${name.padEnd(24)} ${size}x${size}`);
}

const appIco = ico(await Promise.all(ICO_SIZES.map(async (size) => ({ size, data: await png(appSvg(size), size) }))));
await fs.writeFile(path.join(ICONS, "icon.ico"), appIco);
console.log(`${"icon.ico".padEnd(24)} ${ICO_SIZES.join(", ")} (${(appIco.length / 1024).toFixed(0)} KB)`);

const wizardIco = ico(await Promise.all(ICO_SIZES.map(async (size) => ({ size, data: await png(wizardSvg(size), size) }))));
await fs.writeFile(path.join(ICONS, "wizard.ico"), wizardIco);
await fs.writeFile(path.join(ICONS, "wizard.png"), await png(wizardSvg(WIZARD_SIZE), WIZARD_SIZE));
console.log(`${"wizard.ico, wizard.png".padEnd(24)} the wizard's window`);

await browser.close();
