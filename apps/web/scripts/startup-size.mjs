#!/usr/bin/env node
/**
 * What the first page of each route makes the browser download, from the
 * production build (`vite build` first): the scripts and styles the server
 * puts in the page (TanStack Start's route manifest: the root's, then those
 * of every route down to the one opened), compressed with gzip. Chunks a
 * page imports while it runs (strings, background features, a hub card)
 * are not counted: they never hold up the first render.
 *
 *   node scripts/startup-size.mjs            the table
 *   node scripts/startup-size.mjs --check    also fail when a number is over `startup-budget.json`
 *
 * The budget holds the load every page shares (the root) and the heaviest
 * single route. Raise it on purpose, in the PR that needs it, never to make
 * CI pass.
 */
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { gzipSync } from "node:zlib";

const web = join(dirname(fileURLToPath(import.meta.url)), "..");
const server = join(web, ".output/server");
const manifestFile = readdirSync(server).find((name) =>
  name.startsWith("_tanstack-start-manifest"),
);
if (!manifestFile) throw new Error("No build found: run `vite build` first.");
const { routes } = (await import(pathToFileURL(join(server, manifestFile)))).tsrStartManifest();

const sizes = new Map();
function gzipped(path) {
  if (!sizes.has(path)) {
    sizes.set(path, gzipSync(readFileSync(join(web, ".output/public", path))).length);
  }
  return sizes.get(path);
}

/** The files a route adds on top of its parent. */
function own(id) {
  const route = routes[id];
  return { js: route.preloads ?? [], css: route.css ?? [] };
}

const parents = new Map();
for (const [id, route] of Object.entries(routes)) {
  for (const child of route.children ?? []) parents.set(child, id);
}
function chain(id) {
  const ids = [];
  for (let at = id; at; at = parents.get(at)) ids.unshift(at);
  return ids;
}

function load(ids) {
  const js = new Set();
  const css = new Set();
  for (const id of ids) {
    for (const file of own(id).js) js.add(file);
    for (const file of own(id).css) css.add(file);
  }
  const sum = (files) => [...files].reduce((total, file) => total + gzipped(file), 0);
  return { files: js.size, js: sum(js), css: sum(css) };
}

const leaves = Object.keys(routes).filter((id) => !routes[id].children?.length);
const rows = leaves.map((id) => ({ id, ...load(chain(id)) })).sort((a, b) => b.js - a.js);
const root = load(["__root__"]);
const heaviest = rows[0];

const kb = (bytes) => `${(bytes / 1024).toFixed(1)} kB`.padStart(9);
console.log(
  `${"route".padEnd(44)}${"files".padStart(6)}${"js gz".padStart(10)}${"css gz".padStart(10)}`,
);
for (const row of [{ id: "every page (root)", ...root }, ...rows]) {
  console.log(`${row.id.padEnd(44)}${String(row.files).padStart(6)} ${kb(row.js)} ${kb(row.css)}`);
}

if (process.argv.includes("--check")) {
  const budget = JSON.parse(readFileSync(join(web, "startup-budget.json"), "utf8"));
  const over = [
    ["every page (root) js", root.js, budget.rootJsGzipKb],
    [`heaviest route (${heaviest.id}) js`, heaviest.js, budget.routeJsGzipKb],
  ].filter(([, bytes, limit]) => bytes / 1024 > limit);
  for (const [what, bytes, limit] of over) {
    console.error(`over budget: ${what} is ${kb(bytes).trim()}, the budget is ${limit} kB`);
  }
  if (over.length) process.exit(1);
  console.log("\nwithin startup-budget.json");
}
