import { type emoji, toMillis } from "@tokuly/api";
import { useSyncExternalStore } from "react";
import { emojiClient } from "./api";

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

/** A custom emoji as the client keeps it: plain data that survives IndexedDB. */
export interface CatalogEmoji {
  id: string;
  /** The shortcode without colons, lowercase. */
  name: string;
  /** The animated WebP of an animated emoji, the largest still otherwise. */
  url: string;
  /** Still renditions by longest edge, smallest first. */
  stills: { size: number; url: string }[];
  animated: boolean;
  /** Empty when it has none. */
  category: string;
  aliases: string[];
  sensitive: boolean;
  localOnly: boolean;
  /** When it joined the catalogue (ms since the epoch; 0 when unknown). */
  addedAt: number;
}

export interface CatalogCategory {
  name: string;
  emojiCount: number;
}

export interface Catalog {
  /** `idle` before the first load, `loading` until a copy is there, `ready` afterwards (also while refreshing). */
  status: "idle" | "loading" | "ready" | "error";
  version: bigint;
  /** Every approved emoji, ordered by name. */
  emojis: CatalogEmoji[];
  /** Categories in display order. */
  categories: CatalogCategory[];
  byName: ReadonlyMap<string, CatalogEmoji>;
}

export function toCatalogEmoji(message: emoji.CustomEmoji): CatalogEmoji {
  return {
    id: message.id,
    name: message.name,
    url: message.image?.url ?? "",
    stills: (message.image?.variants ?? []).map((variant) => ({ size: variant.size, url: variant.url })).sort((a, b) => a.size - b.size),
    animated: message.animated,
    category: message.category,
    aliases: [...message.aliases],
    sensitive: message.sensitive,
    localOnly: message.localOnly,
    addedAt: message.addedAt ? toMillis(message.addedAt) : 0,
  };
}

const toCategory = (message: emoji.Category): CatalogCategory => ({ name: message.name, emojiCount: message.emojiCount });

function build(version: bigint, emojis: CatalogEmoji[], categories: CatalogCategory[], status: Catalog["status"]): Catalog {
  const sorted = [...emojis].sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
  return { status, version, emojis: sorted, categories, byName: new Map(sorted.map((entry) => [entry.name, entry])) };
}

/** Applies a `GetCatalogChanges` answer to a copy: pure, so it can be tested. */
export function applyChanges(
  current: Pick<Catalog, "version" | "emojis" | "categories">,
  changes: { version: bigint; full: boolean; upserted: CatalogEmoji[]; removedIds: string[]; categories: CatalogCategory[] },
): Catalog {
  const byId = new Map<string, CatalogEmoji>(changes.full ? [] : current.emojis.map((entry) => [entry.id, entry]));
  for (const id of changes.removedIds) byId.delete(id);
  for (const entry of changes.upserted) byId.set(entry.id, entry);
  // The API sends every category whenever the version moved (an empty list then means there are none).
  const changed = changes.full || changes.version !== current.version;
  return build(changes.version, [...byId.values()], changed ? changes.categories : current.categories, "ready");
}

// ───────────────────────────── The copy in IndexedDB ─────────────────────────────

const DB_NAME = "tokuly.emoji";
const STORE = "catalog";
const RECORD = "approved";

interface StoredCatalog {
  /** The version as a decimal string: IndexedDB in older browsers cannot store BigInt. */
  version: string;
  emojis: CatalogEmoji[];
  categories: CatalogCategory[];
}

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, 1);
    request.onupgradeneeded = () => request.result.createObjectStore(STORE);
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

async function readStored(): Promise<StoredCatalog | undefined> {
  try {
    const db = await openDb();
    return await new Promise((resolve, reject) => {
      const request = db.transaction(STORE, "readonly").objectStore(STORE).get(RECORD);
      request.onsuccess = () => resolve(request.result as StoredCatalog | undefined);
      request.onerror = () => reject(request.error);
    });
  } catch {
    // Private mode, a blocked or broken database: the catalogue is simply fetched again.
    return undefined;
  }
}

async function writeStored(catalog: Catalog): Promise<void> {
  try {
    const db = await openDb();
    const record: StoredCatalog = { version: catalog.version.toString(), emojis: catalog.emojis, categories: catalog.categories };
    await new Promise<void>((resolve, reject) => {
      const transaction = db.transaction(STORE, "readwrite");
      transaction.objectStore(STORE).put(record, RECORD);
      transaction.oncomplete = () => resolve();
      transaction.onerror = () => reject(transaction.error);
    });
  } catch {
    // Nothing to keep it in: this visit works from memory.
  }
}

// ───────────────────────────── The copy in memory ─────────────────────────────

let catalog: Catalog = build(0n, [], [], "idle");
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
    const stored = await readStored();
    if (stored) publish(build(BigInt(stored.version), stored.emojis, stored.categories, "ready"));
  }
  try {
    let next: Catalog;
    if (catalog.version === 0n) {
      const whole = await emojiClient().getCatalog({});
      next = build(whole.version, whole.emojis.map(toCatalogEmoji), whole.categories.map(toCategory), "ready");
    } else {
      const changes = await emojiClient().getCatalogChanges({ sinceVersion: catalog.version });
      if (changes.version === catalog.version && !changes.full) {
        if (catalog.status !== "ready") publish({ ...catalog, status: "ready" });
        return;
      }
      next = applyChanges(catalog, {
        version: changes.version,
        full: changes.full,
        upserted: changes.upserted.map(toCatalogEmoji),
        removedIds: changes.removedIds,
        categories: changes.categories.map(toCategory),
      });
    }
    publish(next);
    void writeStored(next);
  } catch (error) {
    // A stored copy keeps working offline; without one the pickers show Unicode emoji only.
    publish({ ...catalog, status: catalog.version > 0n || catalog.emojis.length > 0 ? "ready" : "error" });
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
  if (version !== undefined && catalog.status === "ready" && version <= catalog.version) return Promise.resolve();
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
  const bare = name.startsWith(":") && name.endsWith(":") && name.length > 2 ? name.slice(1, -1) : name;
  return catalog.byName.get(bare.toLowerCase());
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

const SERVER_CATALOG = build(0n, [], [], "idle");

/** The catalogue, loading it on first use; the component draws again whenever it changes. */
export function useCatalog(): Catalog {
  const current = useSyncExternalStore(
    subscribe,
    () => catalog,
    () => SERVER_CATALOG,
  );
  if (typeof window !== "undefined" && current.status === "idle") queueMicrotask(ensureCatalog);
  return current;
}

/** The smallest still that covers `pixels` on a 2x screen (the picture itself when there is none). */
export function stillUrl(entry: CatalogEmoji, pixels: number): string {
  const wanted = pixels * 2;
  return (entry.stills.find((still) => still.size >= wanted) ?? entry.stills[entry.stills.length - 1])?.url ?? entry.url;
}
