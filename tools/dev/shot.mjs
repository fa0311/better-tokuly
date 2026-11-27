#!/usr/bin/env node
// Opens a page of the running web app in Chrome (software WebGL, so the map
// renders without a GPU), optionally signed in, and saves a screenshot.
//
//   node tools/dev/shot.mjs --url /map --out /tmp/map.png --user alice --signup
//
// Options:
//   --url <path|url>      page to open (paths are relative to --base)
//   --out <file.png>      where to save the screenshot
//   --base <origin>       web app origin (default $TOKULY_WEB_URL or http://localhost:3000)
//   --user <handle>       sign in as this user first (session cached in .dev/browser/)
//   --password <pw>       password for --user (default "correct horse battery")
//   --signup              create the account through the API if signing in fails
//   --width/--height      viewport (default 1280x860; try 390x844 for a phone)
//   --dark                dark theme
//   --lang <ja|en>        UI language (default ja)
//   --wait <ms>           settle time before the screenshot (default 2500)
//   --eval <js>           expression evaluated in the page after loading; its JSON result is printed
//   --click <text>        click the first element with this visible text before the screenshot (repeatable)
//   --lat/--lng           mocked device position (default Shibuya)
//   --full                capture the full scrollable page
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const args = process.argv.slice(2);
const flag = (name) => args.includes(`--${name}`);
const value = (name, fallback) => {
  const index = args.indexOf(`--${name}`);
  return index >= 0 ? args[index + 1] : fallback;
};
const values = (name) =>
  args.flatMap((arg, index) => (arg === `--${name}` ? [args[index + 1]] : []));

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const base = value("base", process.env.TOKULY_WEB_URL ?? "http://localhost:3000");
const target = value("url", "/");
const url = target.startsWith("http") ? target : `${base}${target}`;
const out = value("out");
const user = value("user");
const password = value("password", "correct horse battery");
const language = value("lang", "ja");

const browser = await chromium.launch({
  channel: "chrome",
  headless: true,
  // --disable-dev-shm-usage: containers often have a tiny /dev/shm, which crashes renderers.
  args: [
    "--use-angle=swiftshader",
    "--enable-unsafe-swiftshader",
    "--ignore-gpu-blocklist",
    "--enable-webgl",
    "--disable-dev-shm-usage",
  ],
});
const stateDir = path.join(repo, ".dev", "browser");
fs.mkdirSync(stateDir, { recursive: true });
const statePath = user ? path.join(stateDir, `${user}.json`) : undefined;

const context = await browser.newContext({
  viewport: { width: Number(value("width", 1280)), height: Number(value("height", 860)) },
  locale: language === "ja" ? "ja-JP" : "en-US",
  colorScheme: flag("dark") ? "dark" : "light",
  geolocation: {
    latitude: Number(value("lat", 35.6595)),
    longitude: Number(value("lng", 139.7005)),
    accuracy: 12,
  },
  permissions: ["geolocation", "notifications"],
  storageState: statePath && fs.existsSync(statePath) ? statePath : undefined,
});
await context.addCookies([{ name: "tokuly_lang", value: language, url: base }]);
const page = await context.newPage();

const problems = [];
page.on("console", (message) => {
  if (message.type() === "error" && !message.text().includes("Failed to load resource"))
    problems.push(`[console] ${message.text().slice(0, 300)}`);
});
page.on("pageerror", (error) => problems.push(`[pageerror] ${String(error).slice(0, 300)}`));
const failed = new Map();
page.on("response", (response) => {
  if (response.status() < 400) return;
  const key = `${response.status()} ${response
    .url()
    .replace(/\/\d+\/\d+\/\d+\.mvt/, "/z/x/y.mvt")
    .slice(0, 150)}`;
  failed.set(key, (failed.get(key) ?? 0) + 1);
});

async function rpc(procedure, body) {
  const response = await page.request.post(`${base}/api/${procedure}`, {
    data: body,
    headers: { "content-type": "application/json" },
  });
  return { ok: response.ok(), body: await response.json().catch(() => ({})) };
}

async function signIn() {
  await page.goto(`${base}/login`, { waitUntil: "networkidle" });
  await page.locator('input[autocomplete~="username"]').fill(user);
  await page.locator('input[type="password"]').fill(password);
  await page.locator('form button[type="submit"]').click();
  // Signed in once the app remembers the session; do not wait for the (heavy) map to load.
  await page.waitForFunction(() => localStorage.getItem("tokuly.signedIn") === "1", null, {
    timeout: 15000,
  });
}

async function signUp() {
  const created = await rpc("tokuly.identity.v1.AuthService/SignUp", {
    email: `${user}@example.com`,
    password,
    handle: user,
    displayName: user,
    birthdate: "2000-04-02",
    acceptTerms: true,
  });
  if (!created.ok) throw new Error(`sign-up failed: ${JSON.stringify(created.body)}`);
}

async function open() {
  await page.goto(url, { waitUntil: "load" });
  // A signed-in route either keeps its session or bounces to /login. Wait until the app has
  // decided (a cold dev server can take seconds), then for the session restore to finish.
  const decided = () =>
    location.pathname.startsWith("/login") || localStorage.getItem("tokuly.signedIn") === "1";
  await page.waitForFunction(decided, null, { timeout: 10000 }).catch(() => {});
  await page.waitForLoadState("networkidle", { timeout: 5000 }).catch(() => {});
  return (
    !new URL(page.url()).pathname.startsWith("/login") || new URL(url).pathname.startsWith("/login")
  );
}

if (flag("dark")) await page.addInitScript(() => localStorage.setItem("tokuly.theme", "dark"));
let opened = await open();
if (!opened && user) {
  // No usable session (first run, expired, or the database was recreated).
  try {
    await signIn();
  } catch (error) {
    if (!flag("signup")) throw error;
    await signUp();
    await signIn();
  }
  await context.storageState({ path: statePath });
  opened = await open();
}
await page.waitForTimeout(Number(value("wait", 2500)));
for (const text of values("click")) {
  await page.getByText(text, { exact: false }).first().click();
  await page.waitForTimeout(600);
}
const evaluated = value("eval") ? await page.evaluate(value("eval")) : undefined;
if (out) await page.screenshot({ path: out, fullPage: flag("full") });
if (statePath) await context.storageState({ path: statePath });
console.log(
  JSON.stringify(
    {
      url: page.url(),
      eval: evaluated,
      failedRequests: [...failed.entries()].slice(0, 15),
      problems: problems.slice(0, 15),
    },
    null,
    1,
  ),
);
await browser.close();
