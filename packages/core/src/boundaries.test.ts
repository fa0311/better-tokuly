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

/** Globals only a browser (or a native shell) has. The clock is fine. */
const DENIED_GLOBALS =
  /(?<![.\w])(window|document|navigator|localStorage|sessionStorage|indexedDB|Platform\.OS)\b|(?<![.\w])location\.(href|origin|pathname|search|hash|host|reload|assign|replace)\b/;

function sources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return entry.name === "node_modules" ? [] : sources(path);
    return /\.tsx?$/.test(entry.name) && !/\.test\.ts$/.test(entry.name) ? [path] : [];
  });
}

/** The code without comments and string contents, so prose and UI copy may say "window". */
function code(text: string): string {
  return text
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/\/\/.*$/gm, "")
    .replace(/(["'`])(?:\\.|(?!\1)[^\\])*\1/g, '""');
}

const files = PACKAGES.flatMap((dir) => sources(join(dir, "src")));

describe("shared packages", () => {
  it("are found", () => {
    expect(files.length).toBeGreaterThan(10);
  });

  it.each(files.map((file) => [relative(join(import.meta.dirname, "../.."), file), file]))(
    "%s imports nothing platform- or UI-specific",
    (_name, file) => {
      const text = readFileSync(file, "utf8");
      const specifiers = [
        ...text.matchAll(/\bfrom\s+["']([^"']+)["']|\bimport\(\s*["']([^"']+)["']/g),
      ].map((match) => match[1] ?? match[2] ?? "");
      for (const specifier of specifiers)
        expect(
          DENIED_IMPORTS.some((pattern) => pattern.test(specifier)),
          `${specifier} in ${file}`,
        ).toBe(false);
      expect(code(text).match(DENIED_GLOBALS)?.[0], file).toBeUndefined();
    },
  );

  it.each(PACKAGES)("%s type-checks without the DOM library", (dir) => {
    const config = readFileSync(join(dir, "tsconfig.json"), "utf8");
    const lib = /"lib"\s*:\s*\[([^\]]*)\]/.exec(config)?.[1];
    expect(lib, `${dir}/tsconfig.json sets its own "lib"`).toBeDefined();
    expect(lib?.toLowerCase()).not.toContain("dom");
  });
});
