import type { QueryClient } from "@tanstack/query-core";
import type { stamp } from "@tokuly/api";
import { stampKeys } from "./keys";

/** Refetches everything that depends on what the user holds: library lists, store cards, pack pages. */
export function refreshHoldings(queryClient: QueryClient): Promise<unknown> {
  return Promise.all([
    queryClient.invalidateQueries({ queryKey: stampKeys.library }),
    queryClient.invalidateQueries({ queryKey: stampKeys.store }),
    queryClient.invalidateQueries({ queryKey: ["stamp", "pack"] }),
  ]);
}

type Replacer = (message: { $typeName: string }) => unknown;

/**
 * Rebuilds `value` with every protobuf message passed through `replace`
 * (returning `undefined` keeps the message and looks inside it). Untouched
 * branches keep their identity, so only what changed re-renders.
 */
export function mapMessages(value: unknown, replace: Replacer): unknown {
  if (Array.isArray(value)) {
    let changed = false;
    const next = value.map((item) => {
      const mapped = mapMessages(item, replace);
      if (mapped !== item) changed = true;
      return mapped;
    });
    return changed ? next : value;
  }
  if (value === null || typeof value !== "object" || value instanceof Uint8Array) return value;
  if ("$typeName" in value) {
    const replaced = replace(value as { $typeName: string });
    if (replaced !== undefined) return replaced;
  }
  let next: Record<string, unknown> | null = null;
  for (const [key, item] of Object.entries(value)) {
    const mapped = mapMessages(item, replace);
    if (mapped !== item) {
      next ??= { ...(value as Record<string, unknown>) };
      next[key] = mapped;
    }
  }
  return next ?? value;
}

function patchStampCaches(queryClient: QueryClient, replace: Replacer): void {
  for (const [key, data] of queryClient.getQueriesData({ queryKey: stampKeys.all })) {
    const next = mapMessages(data, replace);
    if (next !== data) queryClient.setQueryData(key, next);
  }
}

/** Applies `patch` to one stamp wherever it is cached (pack pages, pickers, lists). */
export function patchStamp(
  queryClient: QueryClient,
  stampId: string,
  patch: Partial<stamp.Stamp>,
): void {
  patchStampCaches(queryClient, (message) => {
    if (message.$typeName !== "tokuly.stamp.v1.Stamp") return undefined;
    const item = message as stamp.Stamp;
    return item.stampRef?.id === stampId ? { ...item, ...patch } : item;
  });
}

/** Applies a change to one pack's summary wherever it is cached (shelves, lists, the pack page). */
export function patchPack(
  queryClient: QueryClient,
  packId: string,
  update: (summary: stamp.PackSummary) => stamp.PackSummary,
): void {
  patchStampCaches(queryClient, (message) => {
    if (message.$typeName !== "tokuly.stamp.v1.PackSummary") return undefined;
    const summary = message as stamp.PackSummary;
    return summary.id === packId ? update(summary) : summary;
  });
}

/** A pack left the library: its cards stop saying "installed" (a purchase stays valid). */
export function markUninstalled(queryClient: QueryClient, packId: string): void {
  patchPack(queryClient, packId, (summary) => ({
    ...summary,
    viewer: summary.viewer && { ...summary.viewer, installed: false },
  }));
}
