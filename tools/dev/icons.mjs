#!/usr/bin/env node
// Draws every raster image the installable web app needs from one source,
// `apps/web/public/icon.svg`, so the icon can change in one place:
//
//   public/icons/icon-{192,512}.png        the icon as it is (rounded square, transparent corners)
//   public/icons/maskable-{192,512}.png    full-bleed background, artwork inside the safe zone
//   public/icons/monochrome-512.png        the pin alone, for themed icons
//   public/icons/badge-96.png              the pin alone, for the status bar next to a notification
//   public/icons/apple-touch-icon.png      180 px, opaque (iOS rounds the corners itself)
//   public/icons/favicon-{16,32}.png       and public/favicon.ico (16, 32, 48)
//   public/icons/shortcut-*.png            the manifest's shortcuts
//   public/splash/iphone-*.png             launch images for iPhones, light and dark
//   src/features/pwa/splash-screens.ts     the <link> each launch image needs
//
//   node tools/dev/icons.mjs                     # write everything
//   node tools/dev/icons.mjs --preview <dir>     # also save sheets to look at: the icons under the
//                                                # masks launchers cut them with, and the launch images
//
// It also packs the manifest's screenshots (public/screenshots/*.png) down to a
// palette, which makes them a third of the size. Those are pictures of the real
// app, taken with tools/dev/shot.mjs against a running stack, signed in as an
// account with something to show (light theme; `--wait` gives the map time to draw):
//
//   node tools/dev/shot.mjs --user <handle> --url /map --width 390 --height 844 --wait 9000 \
//     --out apps/web/public/screenshots/phone-map.png        # also /friends, /stamps
//   node tools/dev/shot.mjs --user <handle> --url /map --width 1280 --height 800 --wait 9000 \
//     --out apps/web/public/screenshots/wide-map.png         # also /stamps
//
// SVG is drawn by Chrome (the same engine the browser tests use), ImageMagick
// (`magick`) packs the results.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const web = path.join(repo, "apps/web");
const publicDir = path.join(web, "public");
const iconsDir = path.join(publicDir, "icons");
const splashDir = path.join(publicDir, "splash");
const splashModule = path.join(web, "src/features/pwa/splash-screens.ts");

const previewIndex = process.argv.indexOf("--preview");
const previewDir =
  previewIndex >= 0 ? path.resolve(process.argv[previewIndex + 1] ?? "") : undefined;

// ── The source icon, taken apart ────────────────────────────────────

const INK = "#17122b";
const source = fs.readFileSync(path.join(publicDir, "icon.svg"), "utf8");
const viewBox = source.match(/viewBox="0 0 (\d+) (\d+)"/);
const body = source.match(/<svg[^>]*>([\s\S]*)<\/svg>/)?.[1];
const plate = body?.match(/<rect[^>]*\/>/)?.[0];
if (!viewBox || viewBox[1] !== viewBox[2] || !body || !plate) {
  throw new Error(
    "icon.svg: expected a square viewBox starting at 0 0, with a background <rect> first",
  );
}
/** Side of the icon's coordinate system. */
const SIZE = Number(viewBox[1]);
const plateFill = plate.match(/fill="([^"]+)"/)?.[1] ?? INK;
/** Everything drawn on the background plate. */
const artwork = body.replace(plate, "").trim();
// The pin and the dot in it, for the single-colour versions.
const pinPath = artwork.match(/<path[^>]*\sd="([^"]+)"[^>]*>/);
const pinEye = artwork
  .slice(pinPath?.index ?? 0)
  .match(/<circle cx="([\d.]+)" cy="([\d.]+)" r="([\d.]+)"/);
if (!pinPath || !pinEye)
  throw new Error("icon.svg: expected the pin <path> followed by the <circle> in it");
const pinOutline = Number(pinPath[0].match(/stroke-width="([\d.]+)"/)?.[1] ?? 0);

const svg = (box, content) =>
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${box}">${content}</svg>`;

/** The icon without its rounded corners: the plate fills the square, the artwork is scaled around the middle. */
function fullBleed(scale, center) {
  const middle = SIZE / 2;
  return svg(
    `0 0 ${SIZE} ${SIZE}`,
    `<rect width="${SIZE}" height="${SIZE}" fill="${plateFill}"/>` +
      `<g transform="translate(${middle} ${middle}) scale(${scale}) translate(${-center.x} ${-center.y})">${artwork}</g>`,
  );
}

/** The pin as one white shape with a hole where its dot is. `fill` is how much of the square it takes up. */
function silhouette(fill, box) {
  const [cx, cy, r] = pinEye.slice(1).map(Number);
  // The outline is part of the shape: it widens the pin and narrows the hole.
  const hole = r + pinOutline / 2;
  const eye = `M${cx - hole} ${cy}a${hole} ${hole} 0 1 0 ${hole * 2} 0a${hole} ${hole} 0 1 0 ${-hole * 2} 0Z`;
  const side = Math.max(box.width, box.height) / fill;
  const origin = { x: box.x + box.width / 2 - side / 2, y: box.y + box.height / 2 - side / 2 };
  return svg(
    `${origin.x} ${origin.y} ${side} ${side}`,
    `<path fill="#fff" fill-rule="evenodd" stroke="#fff" stroke-width="${pinOutline}" stroke-linejoin="round" d="${pinPath[1]} ${eye}"/>`,
  );
}

// ── Shortcut icons ──────────────────────────────────────────────────

/** A round sticker in one of the app's colours with a glyph on it (96 units across). */
function shortcut(tint, glyph) {
  return svg(
    "0 0 96 96",
    `<circle cx="48" cy="48" r="45" fill="${tint}" stroke="${INK}" stroke-width="5"/>${glyph}`,
  );
}
const line = `fill="none" stroke="${INK}" stroke-width="5" stroke-linecap="round" stroke-linejoin="round"`;
const paper = `fill="#fff" stroke="${INK}" stroke-width="5" stroke-linejoin="round"`;
const SHORTCUTS = {
  // The pin of the icon itself, a little larger.
  map: (pin) =>
    shortcut(
      "#c6f24e",
      `<g transform="translate(48 49) scale(1.3) translate(${-pin.x} ${-pin.y})">${pinPath[0]}${pinEye[0]} fill="#fff" stroke="${INK}" stroke-width="3"/></g>`,
    ),
  chat: () =>
    shortcut(
      "#ff5c9d",
      `<path ${paper} d="M36 25h24a12 12 0 0 1 12 12v11a12 12 0 0 1-12 12H47l-13 11V59.800A12 12 0 0 1 24 48V37a12 12 0 0 1 12-12Z"/>` +
        `<g fill="${INK}"><circle cx="37" cy="42.500" r="3.500"/><circle cx="48" cy="42.500" r="3.500"/><circle cx="59" cy="42.500" r="3.500"/></g>`,
    ),
  notes: () =>
    shortcut(
      "#ffd23f",
      `<rect ${paper} x="28" y="22" width="40" height="52" rx="9"/><path ${line} d="M38 38h20M38 49h20M38 60h11"/>`,
    ),
  notifications: () =>
    shortcut(
      "#4fc3ff",
      `<path ${paper} d="M48 22c-9.900 0-17 7.400-17 17v11.500l-5.500 9.500h45L65 50.500V39c0-9.600-7.100-17-17-17Z"/><path ${line} d="M42 68.500a6.300 6.300 0 0 0 12 0"/>`,
    ),
};

// ── Launch images (iOS shows one while the app starts) ──────────────

const THEMES = { light: "#f4f1ff", dark: "#0d0b14" }; // --bg of each theme in src/styles.css
/** The mark the app itself shows while it restores the session (`LogoMark` in src/components/ui/logo.tsx), 80 px. */
const LAUNCH_MARK = svg(
  "0 0 64 64",
  `<circle cx="32" cy="34" r="26" fill="#c6f24e" stroke="${INK}" stroke-width="3"/>` +
    `<path d="M32 8c-9.400 0-17 7.400-17 16.600 0 11.700 13.600 24.300 15.800 26.200a1.800 1.800 0 0 0 2.400 0C35.400 48.900 49 36.300 49 24.600 49 15.400 41.400 8 32 8Z" fill="#7c5cff" stroke="${INK}" stroke-width="3" stroke-linejoin="round"/>` +
    `<circle cx="32" cy="24.500" r="6.500" fill="#fff" stroke="${INK}" stroke-width="3"/>` +
    `<circle cx="46.500" cy="45.500" r="5" fill="#ff5c9d" stroke="${INK}" stroke-width="3"/>`,
);
const LAUNCH_MARK_SIZE = 80;

/**
 * iPhone screens in portrait: CSS pixels, device pixel ratio, and the height
 * of the status bar, which the page does not cover: the app centres its mark
 * in what is left, and the launch image puts it in the same spot.
 */
const IPHONES = [
  { width: 440, height: 956, ratio: 3, statusBar: 62, models: "17 Pro Max, 16 Pro Max" },
  { width: 420, height: 912, ratio: 3, statusBar: 62, models: "Air" },
  { width: 402, height: 874, ratio: 3, statusBar: 62, models: "17 Pro, 17, 16 Pro" },
  {
    width: 430,
    height: 932,
    ratio: 3,
    statusBar: 59,
    models: "16 Plus, 15 Pro Max, 15 Plus, 14 Pro Max",
  },
  { width: 393, height: 852, ratio: 3, statusBar: 59, models: "16, 15 Pro, 15, 14 Pro" },
  { width: 428, height: 926, ratio: 3, statusBar: 47, models: "14 Plus, 13 Pro Max, 12 Pro Max" },
  { width: 390, height: 844, ratio: 3, statusBar: 47, models: "16e, 14, 13 Pro, 13, 12 Pro, 12" },
  { width: 375, height: 812, ratio: 3, statusBar: 47, models: "13 mini, 12 mini, 11 Pro, XS, X" },
  { width: 414, height: 896, ratio: 3, statusBar: 44, models: "11 Pro Max, XS Max" },
  { width: 414, height: 896, ratio: 2, statusBar: 48, models: "11, XR" },
  { width: 414, height: 736, ratio: 3, statusBar: 20, models: "8 Plus" },
  { width: 375, height: 667, ratio: 2, statusBar: 20, models: "SE (2nd and 3rd generation), 8" },
];

function launchPage(phone, background) {
  const left = (phone.width - LAUNCH_MARK_SIZE) / 2;
  const top = phone.statusBar + (phone.height - phone.statusBar - LAUNCH_MARK_SIZE) / 2;
  return `<body style="margin:0;background:${background}"><div style="position:absolute;left:${left}px;top:${top}px;width:${LAUNCH_MARK_SIZE}px;height:${LAUNCH_MARK_SIZE}px">${LAUNCH_MARK}</div></body>`;
}

// ── Drawing ─────────────────────────────────────────────────────────

function magick(...args) {
  try {
    execFileSync("magick", args, { stdio: ["ignore", "ignore", "inherit"] });
  } catch (error) {
    if (error.code === "ENOENT")
      throw new Error("ImageMagick 7 (`magick`) is needed to pack the images");
    throw error;
  }
}

const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "tokuly-icons-"));
const browser = await chromium.launch({
  channel: "chrome",
  headless: true,
  args: ["--disable-dev-shm-usage"],
});
const pages = new Map();
/** A page whose screenshots have `ratio` device pixels per CSS pixel. */
async function pageFor(ratio) {
  if (!pages.has(ratio))
    pages.set(ratio, await (await browser.newContext({ deviceScaleFactor: ratio })).newPage());
  return pages.get(ratio);
}

let scratchCount = 0;
/** Screenshot of `html` at `width` × `height` CSS pixels, as a file in the scratch directory. */
async function shoot(html, width, height, ratio = 1) {
  const page = await pageFor(ratio);
  await page.setViewportSize({ width, height });
  await page.setContent(html);
  const file = path.join(scratch, `${scratchCount++}.png`);
  await page.screenshot({ path: file, omitBackground: true });
  return file;
}

const fill = (markup) =>
  `<body style="margin:0">${markup.replace("<svg ", '<svg style="display:block;width:100vw;height:100vh" ')}</body>`;

/** Draws `markup` (an SVG document) as a square PNG of `size` pixels. */
async function png(markup, size, file, { opaque = false } = {}) {
  const shot = await shoot(fill(markup), size, size);
  // Without transparency the alpha channel goes too: iOS paints transparent pixels black.
  magick(
    shot,
    ...(opaque ? ["-background", plateFill, "-alpha", "remove", "-alpha", "off"] : []),
    "-strip",
    "-define",
    "png:compression-level=9",
    file,
  );
}

/** The box around an SVG fragment's shapes, measured by the browser. */
async function boxOf(fragment) {
  const page = await pageFor(1);
  await page.setContent(
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${SIZE} ${SIZE}"><g id="measured">${fragment}</g></svg>`,
  );
  return page.evaluate(() => {
    const { x, y, width, height } = document.getElementById("measured").getBBox();
    return { x, y, width, height };
  });
}
const centerOf = (box) => ({ x: box.x + box.width / 2, y: box.y + box.height / 2 });

fs.mkdirSync(iconsDir, { recursive: true });
fs.rmSync(splashDir, { recursive: true, force: true });
fs.mkdirSync(splashDir, { recursive: true });

const middle = { x: SIZE / 2, y: SIZE / 2 };
const artCenter = centerOf(await boxOf(artwork));
const pinBox = await boxOf(pinPath[0]);
// With its outline, which the measured box leaves out.
const pinShape = {
  x: pinBox.x - pinOutline / 2,
  y: pinBox.y - pinOutline / 2,
  width: pinBox.width + pinOutline,
  height: pinBox.height + pinOutline,
};

// The icon as drawn.
for (const size of [192, 512]) await png(source, size, path.join(iconsDir, `icon-${size}.png`));
// Maskable: a launcher may cut everything outside a circle of 80% of the width.
// At this scale the artwork ends about a tenth of the width inside that circle.
const MASKABLE_SCALE = 0.86;
for (const size of [192, 512])
  await png(
    fullBleed(MASKABLE_SCALE, artCenter),
    size,
    path.join(iconsDir, `maskable-${size}.png`),
    { opaque: true },
  );
// iOS only rounds the corners, so the artwork keeps the icon's own proportions.
await png(fullBleed(1, middle), 180, path.join(iconsDir, "apple-touch-icon.png"), { opaque: true });
// Single colour: only the shape counts (the system tints it).
await png(silhouette(0.52, pinShape), 512, path.join(iconsDir, "monochrome-512.png"));
await png(silhouette(0.9, pinShape), 96, path.join(iconsDir, "badge-96.png"));
// Favicons for browsers that do not take the SVG.
const favicons = [];
for (const size of [16, 32, 48]) {
  const file =
    size === 48 ? path.join(scratch, "favicon-48.png") : path.join(iconsDir, `favicon-${size}.png`);
  await png(source, size, file);
  favicons.push(file);
}
magick(...favicons, path.join(publicDir, "favicon.ico"));
for (const [name, draw] of Object.entries(SHORTCUTS))
  await png(draw(centerOf(pinShape)), 192, path.join(iconsDir, `shortcut-${name}.png`));

// Launch images, and the links that tell iOS which one belongs to which screen.
const links = [];
for (const phone of IPHONES) {
  for (const [scheme, background] of Object.entries(THEMES)) {
    const name = `iphone-${phone.width * phone.ratio}x${phone.height * phone.ratio}-${scheme}.png`;
    const shot = await shoot(launchPage(phone, background), phone.width, phone.height, phone.ratio);
    // Two flat colours and a small mark: a palette keeps each file to a few kilobytes.
    magick(
      shot,
      "-alpha",
      "off",
      "+dither",
      "-colors",
      "64",
      "-strip",
      "-define",
      "png:compression-level=9",
      path.join(splashDir, name),
    );
    links.push({
      models: phone.models,
      media: `(device-width: ${phone.width}px) and (device-height: ${phone.height}px) and (-webkit-device-pixel-ratio: ${phone.ratio}) and (orientation: portrait) and (prefers-color-scheme: ${scheme})`,
      href: `/splash/${name}`,
    });
  }
}
fs.writeFileSync(
  splashModule,
  `// Written by tools/dev/icons.mjs together with the images in public/splash. Do not edit by hand.

/**
 * The launch image iOS shows while the installed app starts: one per iPhone
 * screen size and colour scheme, picked by \`media\`.
 */
export const SPLASH_SCREENS: readonly { media: string; href: string }[] = [
${links.map((link) => `  // iPhone ${link.models}\n  { media: ${JSON.stringify(link.media)}, href: ${JSON.stringify(link.href)} },`).join("\n")}
];
`,
);

// Screenshots keep their content and size; only the encoding gets smaller.
const screenshotsDir = path.join(publicDir, "screenshots");
const screenshots = fs.existsSync(screenshotsDir)
  ? fs.readdirSync(screenshotsDir).filter((name) => name.endsWith(".png"))
  : [];
for (const name of screenshots) {
  const file = path.join(screenshotsDir, name);
  magick(
    file,
    "-strip",
    "-dither",
    "FloydSteinberg",
    "-colors",
    "255",
    "-define",
    "png:compression-level=9",
    `PNG8:${file}`,
  );
}

// ── Sheets to look at ───────────────────────────────────────────────

if (previewDir) {
  fs.mkdirSync(previewDir, { recursive: true });
  const data = (file) => `data:image/png;base64,${fs.readFileSync(file).toString("base64")}`;
  const icon = (name) => data(path.join(iconsDir, name));
  const tile = (label, content) =>
    `<figure style="margin:0;text-align:center;font:12px sans-serif">${content}<figcaption style="margin-top:6px">${label}</figcaption></figure>`;
  const masked = (name, clip, label) =>
    tile(
      label,
      `<img src="${icon(name)}" width="160" height="160" style="display:block;clip-path:${clip}">`,
    );
  // What a launcher may leave of a maskable icon: the whole square down to a circle of 80% (the safe zone).
  const masks = [
    masked("maskable-512.png", "none", "maskable: full"),
    masked("maskable-512.png", "circle(50%)", "circle"),
    masked("maskable-512.png", "inset(0 round 22%)", "rounded square"),
    masked("maskable-512.png", "circle(40%)", "safe zone only"),
    masked("apple-touch-icon.png", "inset(0 round 22.500%)", "apple-touch-icon as iOS cuts it"),
    masked("icon-512.png", "none", "icon-512"),
  ].join("");
  const onColour = (name, size, background, label) =>
    tile(
      label,
      `<div style="background:${background};padding:16px;border-radius:12px"><img src="${icon(name)}" width="${size}" height="${size}" style="display:block"></div>`,
    );
  const small = [
    onColour("monochrome-512.png", 96, "#444", "monochrome-512"),
    onColour("badge-96.png", 96, "#444", "badge-96"),
    onColour("badge-96.png", 24, "#444", "badge at 24 px"),
    onColour("favicon-32.png", 32, "#fff", "favicon-32"),
    onColour("favicon-16.png", 16, "#fff", "favicon-16"),
    ...Object.keys(SHORTCUTS).map((name) =>
      onColour(`shortcut-${name}.png`, 96, "#e8e8ee", `shortcut-${name}`),
    ),
  ].join("");
  const row = (content) =>
    `<div style="display:flex;gap:20px;align-items:flex-end;padding:20px">${content}</div>`;
  const sheet = await shoot(
    `<body style="margin:0;background:#cfcfd8">${row(masks)}${row(small)}</body>`,
    1120,
    420,
  );
  fs.copyFileSync(sheet, path.join(previewDir, "icons.png"));

  const launches = links
    .filter((link) => /1179x2556|750x1334/.test(link.href))
    .map((link) =>
      tile(
        link.href.replace("/splash/", ""),
        `<img src="${data(path.join(publicDir, link.href))}" height="420" style="display:block;border:1px solid #888">`,
      ),
    )
    .join("");
  fs.copyFileSync(
    await shoot(`<body style="margin:0;background:#cfcfd8">${row(launches)}</body>`, 1000, 500),
    path.join(previewDir, "splash.png"),
  );
}

await browser.close();
fs.rmSync(scratch, { recursive: true, force: true });
console.log(
  `icons: ${fs.readdirSync(iconsDir).length} files in public/icons, ${links.length} launch images in public/splash, ` +
    `${screenshots.length} screenshots packed${previewDir ? `, previews in ${previewDir}` : ""}`,
);
