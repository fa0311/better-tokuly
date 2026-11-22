import type { QueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { emojiClient, emojiKeys } from "./api";
import type { UnicodeEmoji, UnicodeGroup } from "./unicode-data";

// ───────────────────────────── Reporting use ─────────────────────────────

/** `RecordUse` takes at most this many emoji per call. */
const BATCH = 50;
const FLUSH_MS = 1500;

const waiting = new Set<string>();
let timer: ReturnType<typeof setTimeout> | undefined;
let cache: QueryClient | undefined;

/** Called once by the feature's background component, so that reporting can refresh "recently used". */
export function bindUseReporting(queryClient: QueryClient): void {
  cache = queryClient;
}

async function flush(): Promise<void> {
  timer = undefined;
  const batch = [...waiting].slice(0, BATCH);
  for (const item of batch) waiting.delete(item);
  if (waiting.size > 0) timer = setTimeout(flush, FLUSH_MS);
  if (batch.length === 0) return;
  try {
    await emojiClient().recordUse({ emojis: batch });
    void cache?.invalidateQueries({ queryKey: emojiKeys.recent });
  } catch {
    // "Recently used" is a convenience: a report that did not arrive is not worth an error.
  }
}

/**
 * Tells the server which emoji the person just used, so they appear under
 * "recently used" and count towards "popular". Each is written as in text:
 * `:name:` or the Unicode emoji itself. Uses are collected for a moment and
 * sent together. The picker reports what is picked from it; a composer
 * reports what was typed by hand (`shortcodesIn(text)`), after sending.
 */
export function recordEmojiUse(emojis: Iterable<string>): void {
  if (typeof window === "undefined") return;
  for (const item of emojis) if (item) waiting.add(item);
  if (waiting.size > 0 && !timer) timer = setTimeout(flush, FLUSH_MS);
}

// ───────────────────────────── The Unicode table, on demand ─────────────────────────────

export interface UnicodeTable {
  groups: UnicodeGroup[];
  all: UnicodeEmoji[];
}

let table: UnicodeTable | undefined;
let loading: Promise<UnicodeTable> | undefined;

export function loadUnicodeTable(): Promise<UnicodeTable> {
  loading ??= import("./unicode-data").then((module) => {
    table = { groups: module.UNICODE_GROUPS, all: module.UNICODE_EMOJI };
    return table;
  });
  return loading;
}

/** The Unicode emoji with their search words; `undefined` for the instant it takes to load them. */
export function useUnicodeTable(enabled = true): UnicodeTable | undefined {
  const [loaded, setLoaded] = useState(table);
  useEffect(() => {
    if (!enabled || loaded) return;
    let live = true;
    void loadUnicodeTable().then((value) => live && setLoaded(value));
    return () => {
      live = false;
    };
  }, [enabled, loaded]);
  return loaded;
}
