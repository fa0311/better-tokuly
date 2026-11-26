import { emoji } from "@tokuly/api";
import { type Catalog, catalogUpdate, EMPTY_CATALOG } from "@tokuly/core/emoji";
import { create } from "zustand";
import { api } from "#/lib/api";

/**
 * The phone's copy of the emoji catalogue (every approved custom emoji), in
 * memory: read whole once, then only its changes (`@tokuly/core/emoji`),
 * when the app starts and whenever the server announces a newer version.
 * The catalogue is the server's, the same for every account, so it outlives
 * a switch. Drawing never waits for it: until it is there, `:name:` stays
 * text.
 */
export const emojiClient = () => api().service(emoji.EmojiService);

export const useCatalogStore = create<{ catalog: Catalog }>(() => ({ catalog: EMPTY_CATALOG }));

let syncing: Promise<void> | null = null;
/** A version announced while a sync was running: sync once more when it finishes. */
let announced = 0n;

async function sync(): Promise<void> {
  const current = useCatalogStore.getState().catalog;
  if (current.status !== "ready")
    useCatalogStore.setState({ catalog: { ...current, status: "loading" } });
  try {
    const next = await catalogUpdate(emojiClient, useCatalogStore.getState().catalog);
    const latest = useCatalogStore.getState().catalog;
    useCatalogStore.setState({ catalog: next ?? { ...latest, status: "ready" } });
  } catch {
    // Without a copy the pickers have no custom emoji; with one, it keeps working.
    const latest = useCatalogStore.getState().catalog;
    useCatalogStore.setState({
      catalog: { ...latest, status: latest.version > 0n ? "ready" : "error" },
    });
  }
}

/**
 * Brings the copy up to date. Calls made while one runs share it; `version`
 * is the one an announcement named, and a copy that has it is left alone.
 */
export function syncCatalog(version?: bigint): Promise<void> {
  const { catalog } = useCatalogStore.getState();
  if (version !== undefined && catalog.status === "ready" && version <= catalog.version)
    return Promise.resolve();
  if (syncing) {
    if (version !== undefined && version > announced) announced = version;
    return syncing;
  }
  syncing = sync().finally(() => {
    syncing = null;
    if (announced > useCatalogStore.getState().catalog.version) {
      announced = 0n;
      void syncCatalog();
    }
  });
  return syncing;
}

/** The catalogue; the component draws again whenever it changes. */
export function useCatalog(): Catalog {
  return useCatalogStore((state) => state.catalog);
}
