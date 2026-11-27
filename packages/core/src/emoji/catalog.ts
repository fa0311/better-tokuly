import { type emoji, toMillis } from "@tokuly/api";
import type { ClientSource } from "../query";

/**
 * An app's copy of the emoji catalogue (every approved emoji), as plain data:
 * built from `GetCatalog` once, then kept current with `GetCatalogChanges`.
 * Where the copy lives (IndexedDB, memory) and when it is refreshed is the
 * app's; `catalogUpdate` is the one step both do.
 */

/** A custom emoji as the client keeps it: plain data that survives being stored. */
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
    stills: (message.image?.variants ?? [])
      .map((variant) => ({ size: variant.size, url: variant.url }))
      .sort((a, b) => a.size - b.size),
    animated: message.animated,
    category: message.category,
    aliases: [...message.aliases],
    sensitive: message.sensitive,
    localOnly: message.localOnly,
    addedAt: message.addedAt ? toMillis(message.addedAt) : 0,
  };
}

const toCategory = (message: emoji.Category): CatalogCategory => ({
  name: message.name,
  emojiCount: message.emojiCount,
});

export function buildCatalog(
  version: bigint,
  emojis: CatalogEmoji[],
  categories: CatalogCategory[],
  status: Catalog["status"],
): Catalog {
  const sorted = [...emojis].sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
  return {
    status,
    version,
    emojis: sorted,
    categories,
    byName: new Map(sorted.map((entry) => [entry.name, entry])),
  };
}

/** The copy before anything was loaded. */
export const EMPTY_CATALOG: Catalog = buildCatalog(0n, [], [], "idle");

/** Applies a `GetCatalogChanges` answer to a copy: pure, so it can be tested. */
export function applyChanges(
  current: Pick<Catalog, "version" | "emojis" | "categories">,
  changes: {
    version: bigint;
    full: boolean;
    upserted: CatalogEmoji[];
    removedIds: string[];
    categories: CatalogCategory[];
  },
): Catalog {
  const byId = new Map<string, CatalogEmoji>(
    changes.full ? [] : current.emojis.map((entry) => [entry.id, entry]),
  );
  for (const id of changes.removedIds) byId.delete(id);
  for (const entry of changes.upserted) byId.set(entry.id, entry);
  // The API sends every category whenever the version moved (an empty list then means there are none).
  const changed = changes.full || changes.version !== current.version;
  return buildCatalog(
    changes.version,
    [...byId.values()],
    changed ? changes.categories : current.categories,
    "ready",
  );
}

/**
 * Asks the server for what is new since `current`: the whole catalogue for a
 * copy that has none yet, the changes otherwise. `null` when nothing changed.
 */
export async function catalogUpdate(
  client: ClientSource<typeof emoji.EmojiService>,
  current: Catalog,
): Promise<Catalog | null> {
  if (current.version === 0n) {
    const whole = await client().getCatalog({});
    return buildCatalog(
      whole.version,
      whole.emojis.map(toCatalogEmoji),
      whole.categories.map(toCategory),
      "ready",
    );
  }
  const changes = await client().getCatalogChanges({ sinceVersion: current.version });
  if (changes.version === current.version && !changes.full) return null;
  return applyChanges(current, {
    version: changes.version,
    full: changes.full,
    upserted: changes.upserted.map(toCatalogEmoji),
    removedIds: changes.removedIds,
    categories: changes.categories.map(toCategory),
  });
}

/** The approved emoji with this name (any case, with or without colons), or `undefined`. */
export function findEmoji(catalog: Catalog, name: string): CatalogEmoji | undefined {
  const bare =
    name.startsWith(":") && name.endsWith(":") && name.length > 2 ? name.slice(1, -1) : name;
  return catalog.byName.get(bare.toLowerCase());
}

/** The smallest still that covers `pixels` on a screen of this density (the picture itself when there is none). */
export function stillUrl(entry: CatalogEmoji, pixels: number, density = 2): string {
  const wanted = pixels * density;
  return (
    (entry.stills.find((still) => still.size >= wanted) ?? entry.stills[entry.stills.length - 1])
      ?.url ?? entry.url
  );
}
