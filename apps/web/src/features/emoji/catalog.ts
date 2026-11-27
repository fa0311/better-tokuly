import {
  buildCatalog,
  type Catalog,
  type CatalogCategory,
  type CatalogEmoji,
  catalogUpdate,
  EMPTY_CATALOG,
  findEmoji,
} from "@tokuly/core/emoji";
import { useSyncExternalStore } from "react";
import { emojiClient } from "./api";
import { createCatalogStore } from "./catalog-store";

/**
 * This browser's copy of the emoji catalogue.
 *
 * The approved set is fetched once (`GetCatalog`), kept in IndexedDB with the
 * version the API returned, and from then on only changes are asked for
 * (`GetCatalogChanges`): when the app starts and whenever the realtime topic
 * `emoji:catalog` announces a newer version. Rendering never waits for the
 * network: `lookupEmoji(name)` is a synchronous map read, and components that
 * use `useCatalog()` draw again when the copy changes.
 */

// ───────────────────────────── The copy in IndexedDB ─────────────────────────────

const store = createCatalogStore<CatalogEmoji, CatalogCategory>();

// ───────────────────────────── The copy in memory ─────────────────────────────

let catalog: Catalog = EMPTY_CATALOG;
const listeners = new Set<() => void>();
let syncing: Promise<void> | null = null;
/** A version announced while a sync was running: sync once more when it finishes. */
let announced = 0n;

function publish(next: Catalog): void {
  catalog = next;
  for (const listener of listeners) listener();
}

async function sync(): Promise<void> {
  if (catalog.status === "idle" || catalog.status === "error") {
    publish({ ...catalog, status: "loading" });
    // Gives up after a moment (see `catalog-store.ts`): a database that does not answer must not hold up the fetch.
    const stored = await store.read();
    if (stored)
      publish(buildCatalog(BigInt(stored.version), stored.emojis, stored.categories, "ready"));
  }
  try {
    const next = await catalogUpdate(emojiClient, catalog);
    if (!next) {
      if (catalog.status !== "ready") publish({ ...catalog, status: "ready" });
      return;
    }
    publish(next);
    void store.write({
      version: next.version.toString(),
      emojis: next.emojis,
      categories: next.categories,
    });
  } catch (error) {
    // A stored copy keeps working offline; without one the pickers show Unicode emoji only.
    publish({
      ...catalog,
      status: catalog.version > 0n || catalog.emojis.length > 0 ? "ready" : "error",
    });
    throw error;
  }
}

/**
 * Brings the copy up to date. Safe to call from anywhere, any number of
 * times: calls made while one is running share it. `version` is the one a
 * realtime announcement named; a copy that already has it is left alone.
 */
export function syncCatalog(version?: bigint): Promise<void> {
  if (typeof window === "undefined") return Promise.resolve();
  if (version !== undefined && catalog.status === "ready" && version <= catalog.version)
    return Promise.resolve();
  if (syncing) {
    if (version !== undefined && version > announced) announced = version;
    return syncing;
  }
  syncing = sync()
    .catch(() => {})
    .finally(() => {
      syncing = null;
      if (announced > catalog.version) {
        announced = 0n;
        void syncCatalog();
      }
    });
  return syncing;
}

/** Loads the catalogue if nothing did yet. */
export function ensureCatalog(): void {
  if (catalog.status === "idle") void syncCatalog();
}

/** The copy as it is right now. */
export function getCatalog(): Catalog {
  return catalog;
}

/** The approved emoji with this name (any case, with or without colons), or `undefined`. */
export function lookupEmoji(name: string): CatalogEmoji | undefined {
  return findEmoji(catalog, name);
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** The catalogue, loading it on first use; the component draws again whenever it changes. */
export function useCatalog(): Catalog {
  const current = useSyncExternalStore(
    subscribe,
    () => catalog,
    () => EMPTY_CATALOG,
  );
  if (typeof window !== "undefined" && current.status === "idle") queueMicrotask(ensureCatalog);
  return current;
}
