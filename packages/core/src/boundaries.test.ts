import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * The shared packages hold logic only, for every app (web, mobile, desktop):
 * nothing in them may reach for a platform or for UI. See packages/README.md.
 */
const PACKAGES = ["core", "i18n"].map((name) => join(import.meta.dirname, "../..", name));

/** Imports that belong to one platform, to UI, or to an app. */
const DENIED_IMPORTS = [
  /^react$/,
  /^react\//,
  /^react-dom/,
  /^react-native/,
  /^react-i18next/,
  /^expo/,
  /^@tanstack\/react-/,
  /^@tauri-apps\//,
  /^zustand/,
  /^sonner/,
  /^#\//,
  /(^|\/)apps\//,
];

/**
 * Globals only a browser (or a native shell) has. The clock is fine. `globalThis`
 * and `self` are denied outright: shared code has no reason to reach for the
 * global object, and through it any of these is one cast away.
 */
const DENIED_GLOBALS =
  /(?<![.\w$])(window|document|navigator|localStorage|sessionStorage|indexedDB|globalThis|self|Platform\.OS)\b|(?<![.\w$])location\.(href|origin|pathname|search|hash|host|reload|assign|replace)\b/;

/** A triple-slash directive that pulls the DOM (or any other) library back into the program. */
const LIB_REFERENCE = /^\s*\/\/\/\s*<reference\s+lib\s*=/m;

function sources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return entry.name === "node_modules" ? [] : sources(path);
    // This file holds the forbidden things as probes, in strings.
    return /\.tsx?$/.test(entry.name) && path !== import.meta.filename ? [path] : [];
  });
}

/** The code without comments and string contents, so prose and UI copy may say "window". */
function code(text: string): string {
  return text
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/\/\/.*$/gm, "")
    .replace(/(["'`])(?:\\.|(?!\1)[^\\])*\1/g, '""');
}

/** Every module a file loads: `from`, side-effect `import "x"`, `import("x")`, `require("x")`. */
function specifiers(text: string): string[] {
  return [
    ...text.matchAll(
      /\bfrom\s+["']([^"']+)["']|\bimport\s+["']([^"']+)["']|\b(?:import|require)\(\s*["']([^"']+)["']/g,
    ),
  ].map((match) => match[1] ?? match[2] ?? match[3] ?? "");
}

/**
 * What in a source file breaks the rules of a shared package. Tests are held
 * to the same rules: a test's `/// <reference lib>` would put the DOM types
 * into the whole package's type check.
 */
function violations(text: string): string[] {
  const found = specifiers(text).filter((specifier) =>
    DENIED_IMPORTS.some((pattern) => pattern.test(specifier)),
  );
  const global = code(text).match(DENIED_GLOBALS)?.[0];
  if (global) found.push(global);
  if (LIB_REFERENCE.test(text)) found.push("/// <reference lib>");
  return found;
}

const files = PACKAGES.flatMap((dir) => sources(join(dir, "src")));

describe("shared packages", () => {
  it("are found", () => {
    expect(files.length).toBeGreaterThan(10);
  });

  it.each(files.map((file) => [relative(join(import.meta.dirname, "../.."), file), file]))(
    "%s uses nothing platform- or UI-specific",
    (_name, file) => {
      expect(violations(readFileSync(file, "utf8"))).toEqual([]);
    },
  );

  it.each([
    ['import { useState } from "react";', "react"],
    ['import "react-dom";', "react-dom"],
    ['const C = await import("react-native");', "react-native"],
    ['import { x } from "#/lib/api";', "#/lib/api"],
    ["export const w = () => window.innerWidth;", "window"],
    ["export const w = () => (globalThis as any).window;", "globalThis"],
    ["export const n = () => (globalThis as { navigator?: unknown }).navigator;", "globalThis"],
    ["export const s = () => self.localStorage;", "self"],
    ['/// <reference lib="dom" />\nexport {};', "/// <reference lib>"],
  ])("denies %s", (source, what) => {
    expect(violations(source)).toContain(what);
  });

  it("allows what reads like a platform only in words", () => {
    expect(
      violations(
        '// the window of slides\nexport const label = "location";\nconst a = b.location;',
      ),
    ).toEqual([]);
  });

  it.each(PACKAGES)("%s type-checks without the DOM library", (dir) => {
    const config = readFileSync(join(dir, "tsconfig.json"), "utf8");
    const lib = /"lib"\s*:\s*\[([^\]]*)\]/.exec(config)?.[1];
    expect(lib, `${dir}/tsconfig.json sets its own "lib"`).toBeDefined();
    expect(lib?.toLowerCase()).not.toContain("dom");
  });
});
