// Writes the seam registries: for every seam, the list of the files the features plug into it
// (`src/features/<area>/<seam>.ts(x)` → `src/generated/seams/<seam>.ts`). Metro has no
// `import.meta.glob`, so the list the web app gets from Vite is written here instead. Not checked
// in (two branches that both add a feature would always conflict there): metro.config.js runs
// this for every bundle and dev server, the typecheck and test scripts before they start.
const fs = require("node:fs");
const path = require("node:path");

const APP = path.resolve(__dirname, "..");
const FEATURES = path.join(APP, "src", "features");
const OUTPUT = path.join(APP, "src", "generated", "seams");

/** The seams, by the file name a feature uses; each is documented in `src/seams/<file>`. */
const SEAMS = [
  "background",
  "badges",
  "forget-user",
  "hub",
  "lifecycle",
  "links",
  "profile",
  "screens",
  "settings-card",
  "settings-rows",
  "tabs",
];

/** The features that plug into `seam`, by name. */
function areasOf(seam) {
  return fs
    .readdirSync(FEATURES)
    .filter((area) =>
      [".ts", ".tsx"].some((extension) =>
        fs.existsSync(path.join(FEATURES, area, seam + extension)),
      ),
    )
    .sort();
}

/** Every file under `folder`, as a path from it. */
function filesUnder(folder) {
  return fs
    .readdirSync(folder, { withFileTypes: true })
    .flatMap((entry) =>
      entry.isDirectory()
        ? filesUnder(path.join(folder, entry.name)).map((file) => path.join(entry.name, file))
        : [entry.name],
    );
}

/**
 * Why `file` (a path from the features folder) carries a seam's name but
 * would not plug in: it is anywhere but `<area>/<seam>.ts(x)`, or has no
 * default export (`source` reads it). `null` for every other file.
 */
function strayReason(file, source) {
  const { dir, name, ext } = path.parse(file);
  if (!SEAMS.includes(name) || !/^\.[cm]?[jt]sx?$/.test(ext)) return null;
  if (dir === "" || dir.includes(path.sep) || ![".ts", ".tsx"].includes(ext))
    return `a seam file is src/features/<area>/${name}.ts(x)`;
  return /^export default\b/m.test(source()) ? null : "no `export default`";
}

/**
 * The seam files no registry would list. Such a file is left out without a
 * sound — for `lifecycle`, an account that leaves with its position still
 * shared — so it stops the build instead.
 */
function strayFiles() {
  return filesUnder(FEATURES).flatMap((file) => {
    const reason = strayReason(file, () => fs.readFileSync(path.join(FEATURES, file), "utf8"));
    return reason ? [`${file}: ${reason}`] : [];
  });
}

const identifier = (area) => area.replace(/-(.)/g, (_, letter) => letter.toUpperCase());

/** The registry of one seam: every feature's default export, in the order of the areas' names. */
function registrySource(seam) {
  const areas = areasOf(seam);
  return [
    `// Written by scripts/seams.js from src/features/*/${seam}.ts(x). Do not edit: \`pnpm seams\`.`,
    ...areas.map((area) => `import ${identifier(area)} from "#/features/${area}/${seam}";`),
    "",
    `export default [${areas.map(identifier).join(", ")}];`,
    "",
  ].join("\n");
}

const registryFile = (seam) => path.join(OUTPUT, `${seam}.ts`);

/** The seams whose registry on disk is not what the features' files say right now. */
function staleRegistries() {
  return SEAMS.filter((seam) => {
    const file = registryFile(seam);
    return !fs.existsSync(file) || fs.readFileSync(file, "utf8") !== registrySource(seam);
  });
}

/**
 * Writes the registries that changed (an untouched file does not wake Metro or tsc). Throws,
 * and with it stops the bundle, type check or test run, when a seam file would be left out.
 */
function writeRegistries() {
  const strays = strayFiles();
  if (strays.length > 0)
    throw new Error(`Seam files that no registry would list:\n  ${strays.join("\n  ")}`);
  fs.mkdirSync(OUTPUT, { recursive: true });
  for (const seam of staleRegistries()) fs.writeFileSync(registryFile(seam), registrySource(seam));
}

module.exports = { SEAMS, areasOf, staleRegistries, strayFiles, strayReason, writeRegistries };

if (require.main === module) writeRegistries();
