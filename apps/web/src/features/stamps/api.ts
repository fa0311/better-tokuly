import { type QueryClient, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { isApiError, stamp } from "@tokuly/api";
import { toast } from "sonner";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import i18n from "#/lib/i18n";

export const storeClient = () => api().service(stamp.StampStoreService);
export const libraryClient = () => api().service(stamp.StampLibraryService);
export const studioClient = () => api().service(stamp.StampStudioService);
export const reviewClient = () => api().service(stamp.StampReviewService);

/** Query keys of the stamp domain. Everything starts with `"stamp"`, so one invalidation reaches all of it. */
export const stampKeys = {
  all: ["stamp"] as const,
  store: ["stamp", "store"] as const,
  storeHome: ["stamp", "store", "home"] as const,
  storeList: (section: stamp.StoreSection, tag: string, price: stamp.PriceFilter) =>
    ["stamp", "store", "list", section, tag, price] as const,
  storeSearch: (query: string, price: stamp.PriceFilter) =>
    ["stamp", "store", "search", query, price] as const,
  tags: ["stamp", "store", "tags"] as const,
  creator: (userId: string) => ["stamp", "store", "creator", userId] as const,
  pack: (packId: string) => ["stamp", "pack", packId] as const,
  stamp: (stampId: string) => ["stamp", "stamp", stampId] as const,
  library: ["stamp", "library"] as const,
  libraryHome: ["stamp", "library", "home"] as const,
  installed: ["stamp", "library", "installed"] as const,
  purchased: ["stamp", "library", "purchased"] as const,
  favorites: ["stamp", "library", "favorites"] as const,
  recents: (order: stamp.RecentOrder) => ["stamp", "library", "recents", order] as const,
  personal: ["stamp", "library", "personal"] as const,
  librarySearch: (query: string) => ["stamp", "library", "search", query] as const,
  usageLog: ["stamp", "library", "log"] as const,
  studio: ["stamp", "studio"] as const,
  creatorProfile: ["stamp", "studio", "profile"] as const,
  myPacks: (official: boolean) => ["stamp", "studio", "packs", official] as const,
  packStats: (packId: string, days: number) => ["stamp", "studio", "stats", packId, days] as const,
  review: ["stamp", "review"] as const,
  reviewQueue: ["stamp", "review", "queue"] as const,
};

/** Refetches everything that depends on what the user holds: library lists, store cards, pack pages. */
export function refreshHoldings(queryClient: QueryClient): Promise<unknown> {
  return Promise.all([
    queryClient.invalidateQueries({ queryKey: stampKeys.library }),
    queryClient.invalidateQueries({ queryKey: stampKeys.store }),
    queryClient.invalidateQueries({ queryKey: ["stamp", "pack"] }),
  ]);
}

// ───────────────────────────── Cache patching ─────────────────────────────

type Replacer = (message: { $typeName: string }) => unknown;

/**
 * Rebuilds `value` with every protobuf message passed through `replace`
 * (returning `undefined` keeps the message and looks inside it). Untouched
 * branches keep their identity, so only what changed re-renders.
 */
function mapMessages(value: unknown, replace: Replacer): unknown {
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

// ───────────────────────────── Library ─────────────────────────────

/** Everything a picker needs in one call: installed packs, recents, favourites, personal stamps. */
export function useLibrary(options?: { refetchOnMount?: "always" }) {
  return useQuery({
    queryKey: stampKeys.libraryHome,
    queryFn: () => libraryClient().getLibrary({}),
    refetchOnMount: options?.refetchOnMount,
  });
}

/** A pack with its stamps (UUID or short ID). */
export function usePack(packId: string | undefined) {
  return useQuery({
    queryKey: stampKeys.pack(packId ?? ""),
    queryFn: async () => (await storeClient().getPack({ packId: packId! })).pack!,
    enabled: Boolean(packId),
  });
}

/** Adds a (free or already bought) pack to the library. */
export function useInstallPack() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (packId: string) => libraryClient().installPack({ packId }),
    onSuccess: (response) => {
      const installed = response.pack;
      if (installed) patchPack(queryClient, installed.id, () => installed);
      toast.success(i18n.t("toast.installed", { ns: "stamps" }));
      return refreshHoldings(queryClient);
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "stamps" })),
  });
}

/** Removes a pack from the library (a purchase stays valid). */
export function useUninstallPack() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (packId: string) => libraryClient().uninstallPack({ packId }),
    onSuccess: (_, packId) => {
      patchPack(queryClient, packId, (summary) => ({
        ...summary,
        viewer: summary.viewer && { ...summary.viewer, installed: false },
      }));
      toast.success(i18n.t("toast.uninstalled", { ns: "stamps" }));
      return refreshHoldings(queryClient);
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "stamps" })),
  });
}

/** Flips a stamp's favourite mark; the star changes at once and rolls back if the server says no. */
export function useToggleFavorite() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({ stampId, favorited }: { stampId: string; favorited: boolean }) => {
      if (favorited) await libraryClient().favoriteStamp({ stampId });
      else await libraryClient().unfavoriteStamp({ stampId });
    },
    onMutate: ({ stampId, favorited }) => patchStamp(queryClient, stampId, { favorited }),
    onError: (error, { stampId, favorited }) => {
      patchStamp(queryClient, stampId, { favorited: !favorited });
      toast.error(errorMessage(error, { namespace: "stamps" }));
    },
    onSettled: () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: stampKeys.favorites }),
        queryClient.invalidateQueries({ queryKey: stampKeys.libraryHome }),
      ]),
  });
}

// ───────────────────────────── Uploads ─────────────────────────────

/**
 * What to tell someone whose stamp upload failed. An upload the server
 * accepted but could not process fails with a plain error named after the
 * asset's failure code (`ANIMATION_TOO_LARGE`…), which gets its own wording.
 */
export function uploadErrorMessage(error: unknown): string {
  if (error instanceof Error && !isApiError(error) && /^[A-Z_]+$/.test(error.message)) {
    const key = `uploadErrors.${error.message}`;
    return i18n.t(i18n.exists(key, { ns: "stamps" }) ? key : "uploadErrors.generic", {
      ns: "stamps",
    });
  }
  return errorMessage(error, { namespace: "stamps" });
}

// ───────────────────────────── Formatting ─────────────────────────────

/** A count for humans: "1.2万" / "12K". */
export function formatCount(value: bigint | number): string {
  return new Intl.NumberFormat(i18n.language, {
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(value);
}

/** The i18n key (in the `stamps` namespace) naming a pack status. */
export function statusKey(status: stamp.PackStatus): string {
  switch (status) {
    case stamp.PackStatus.IN_REVIEW:
      return "status.inReview";
    case stamp.PackStatus.PUBLISHED:
      return "status.published";
    case stamp.PackStatus.REJECTED:
      return "status.rejected";
    case stamp.PackStatus.REMOVED:
      return "status.removed";
    default:
      return "status.draft";
  }
}

/** The i18n key (in the `stamps` namespace) naming where a stamp was sent. */
export function contextKey(context: stamp.UsageContext): string {
  switch (context) {
    case stamp.UsageContext.CHAT:
      return "context.chat";
    case stamp.UsageContext.LIVE_CHAT:
      return "context.liveChat";
    case stamp.UsageContext.COMMENT:
      return "context.comment";
    case stamp.UsageContext.NOTE:
      return "context.note";
    case stamp.UsageContext.MAP:
      return "context.map";
    default:
      return "context.unknown";
  }
}
