import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { expect, it } from "vitest";
import en from "./en/errors";
import ja from "./ja/errors";

const repo = join(import.meta.dirname, "../../..");

function files(dir: string, extension: string): string[] {
  return readdirSync(dir, { recursive: true, encoding: "utf8" })
    .filter((name) => name.endsWith(extension))
    .map((name) => join(dir, name));
}

const REASON = /\b[A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+\b/g;

/**
 * The reasons the protos document: every `Errors: …` (and "Reasons every RPC
 * may return …: …") in a comment, to the end of that comment. Words in
 * parentheses explain a reason and are left out, as are enum values.
 */
function documentedReasons(): Set<string> {
  const reasons = new Set<string>();
  for (const file of files(join(repo, "proto"), ".proto")) {
    const text = readFileSync(file, "utf8");
    const values = new Set(
      [...text.matchAll(/^\s*([A-Z][A-Z0-9_]*)\s*=\s*\d+/gm)].map((m) => m[1]),
    );
    const comments = text.match(/(?:^[ \t]*\/\/.*\n?)+/gm) ?? [];
    for (const block of comments) {
      const comment = block.replace(/^[ \t]*\/\/ ?/gm, "").replace(/\s+/g, " ");
      const start = comment.search(/\bErrors:|\bReasons [^:.]*:/);
      if (start < 0) continue;
      const listed = comment.slice(start).replace(/\([^()]*\)/g, "");
      for (const [reason] of listed.matchAll(REASON)) if (!values.has(reason)) reasons.add(reason);
    }
  }
  return reasons;
}

/** The reasons the backend raises by name: `AppError::<kind>("REASON", …)` and `ensure!(…, kind, "REASON", …)`. */
function raisedReasons(): Set<string> {
  const reasons = new Set<string>();
  for (const file of files(join(repo, "backend/crates"), ".rs")) {
    if (!file.includes("/src/")) continue;
    const code = readFileSync(file, "utf8").split("#[cfg(test)]")[0] ?? "";
    for (const [, reason] of code.matchAll(
      /AppError::\w+\(\s*(?:(?:connectrpc::)?ErrorCode::\w+,\s*)?"([A-Z][A-Z0-9_]*)"/g,
    ))
      reasons.add(reason ?? "");
    for (const [, reason] of code.matchAll(/ensure!\([^;]*?,\s*\w+,\s*"([A-Z][A-Z0-9_]*)"/g))
      reasons.add(reason ?? "");
  }
  return reasons;
}

// The app shows any of them wherever it happens, whatever strings it has loaded (`@tokuly/core/errors`).
it("has a text for every reason the API can return, in both languages", () => {
  const reasons = [...new Set([...documentedReasons(), ...raisedReasons()])].sort();
  expect(reasons.length).toBeGreaterThan(900);
  expect(reasons.filter((reason) => !(reason in ja))).toEqual([]);
  expect(reasons.filter((reason) => !(reason in en))).toEqual([]);
});
