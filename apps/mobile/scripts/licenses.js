// Writes the open-source notices the app shows (Settings → licences) from what the build
// bundles: every production dependency of the app, found the way Node resolves it (through
// pnpm's links), with its licence and licence text; plus the native libraries our own local
// modules add (`modules/*/notices/notices.json`). Run by app.config.ts, so every build, export
// and dev server has them, and by the typecheck and test scripts.
const fs = require("node:fs");
const path = require("node:path");

const APP = path.resolve(__dirname, "..");
const OUTPUT = path.join(APP, "src", "generated", "licenses.json");
const LICENCE_FILE = /^(licen[cs]e|copying|notice)(\.[a-z0-9]+)?$/i;

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

/** The directory of package `name` as seen from `fromDir`, the way Node looks it up. */
function findPackage(fromDir, name) {
  let dir = fromDir;
  for (;;) {
    const candidates = [path.join(dir, "node_modules", name)];
    if (path.basename(dir) === "node_modules") candidates.push(path.join(dir, name));
    for (const candidate of candidates)
      if (fs.existsSync(path.join(candidate, "package.json"))) return fs.realpathSync(candidate);
    const parent = path.dirname(dir);
    if (parent === dir) return null;
    dir = parent;
  }
}

function licenceOf(manifest) {
  if (typeof manifest.license === "string") return manifest.license;
  if (manifest.license?.type) return manifest.license.type;
  if (Array.isArray(manifest.licenses))
    return manifest.licenses.map((entry) => entry.type ?? entry).join(" OR ");
  return "UNKNOWN";
}

function urlOf(manifest) {
  const repository =
    typeof manifest.repository === "string" ? manifest.repository : manifest.repository?.url;
  const url = manifest.homepage ?? repository ?? "";
  return url.replace(/^git\+/, "").replace(/\.git$/, "");
}

function licenceText(dir) {
  const files = fs
    .readdirSync(dir)
    .filter((file) => LICENCE_FILE.test(file))
    .sort();
  return files.map((file) => fs.readFileSync(path.join(dir, file), "utf8").trim()).join("\n\n");
}

/** Every package the app ships, by `name@version`, its own workspace packages left out (but walked through). */
function collect() {
  const found = new Map();
  const seen = new Set();
  const visit = (dir) => {
    if (seen.has(dir)) return;
    seen.add(dir);
    const manifest = readJson(path.join(dir, "package.json"));
    if (dir !== APP && !manifest.private) {
      const key = `${manifest.name}@${manifest.version}`;
      if (!found.has(key))
        found.set(key, {
          name: manifest.name,
          version: manifest.version,
          license: licenceOf(manifest),
          url: urlOf(manifest),
          text: licenceText(dir),
        });
    }
    const dependencies = {
      ...manifest.dependencies,
      ...manifest.optionalDependencies,
    };
    for (const name of Object.keys(dependencies)) {
      const child = findPackage(dir, name);
      if (child) visit(child);
    }
  };
  visit(APP);
  return [...found.values()];
}

/** The native libraries of the app's own modules, which no package.json names. */
function nativeNotices() {
  const modules = path.join(APP, "modules");
  if (!fs.existsSync(modules)) return [];
  return fs.readdirSync(modules).flatMap((module) => {
    const notices = path.join(modules, module, "notices");
    const list = path.join(notices, "notices.json");
    if (!fs.existsSync(list)) return [];
    return readJson(list).map(({ textFile, ...entry }) => ({
      ...entry,
      text: fs.readFileSync(path.join(notices, textFile), "utf8").trim(),
    }));
  });
}

/** Writes `src/generated/licenses.json`: the packages, and each distinct licence text once. */
function writeLicenses() {
  const all = [...collect(), ...nativeNotices()].sort(
    (a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version),
  );
  const texts = [];
  const index = new Map();
  const packages = all.map(({ text, ...entry }) => {
    if (!text) return { ...entry, text: -1 };
    if (!index.has(text)) {
      index.set(text, texts.length);
      texts.push(text);
    }
    return { ...entry, text: index.get(text) };
  });
  const content = `${JSON.stringify({ packages, texts })}\n`;
  fs.mkdirSync(path.dirname(OUTPUT), { recursive: true });
  if (!fs.existsSync(OUTPUT) || fs.readFileSync(OUTPUT, "utf8") !== content)
    fs.writeFileSync(OUTPUT, content);
  return packages.length;
}

module.exports = { writeLicenses };

if (require.main === module) console.log(`licences: ${writeLicenses()} packages`);
