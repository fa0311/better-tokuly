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

/** Writes the registries that changed (an untouched file does not wake Metro or tsc). */
function writeRegistries() {
  fs.mkdirSync(OUTPUT, { recursive: true });
  for (const seam of staleRegistries()) fs.writeFileSync(registryFile(seam), registrySource(seam));
}

module.exports = { SEAMS, areasOf, staleRegistries, writeRegistries };

if (require.main === module) writeRegistries();
