import { Dialog as BaseDialog } from "@base-ui/react/dialog";
import { Popover } from "@base-ui/react/popover";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { common, stamp } from "@tokuly/api";
import { stampKeys } from "@tokuly/core/stamps";
import { Clock, Heart, Plus, Scissors, Store } from "lucide-react";
import {
  type ReactElement,
  type ReactNode,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { libraryClient, useLibrary, usePack } from "./api";
import { PersonalStampMaker } from "./editor/stamp-maker";
import { PackCover, SearchField } from "./parts";
import { StampImage } from "./stamp-image";
import { StampSheet } from "./stamp-sheet";

export interface StampPickerProps {
  /**
   * Called with the stamp the person tapped. Send it (`stamp.id` is what the
   * chat, comment, note and map APIs take); the server records the use, which
   * is what fills "recently used".
   */
  onPick(stamp: common.StampRef): void;
  /**
   * The element that opens the picker, usually an icon button. It must
   * forward its props and ref, which `Button` and plain DOM elements do.
   */
  trigger: ReactElement<Record<string, unknown>>;
  /** Keep the picker open after a pick, to send several stamps in a row. Default: it closes. */
  keepOpen?: boolean;
}

/**
 * The stamp picker: recently used, favourites, personal stamps and every
 * installed pack as tabs, a search across all of them, and a shortcut to the
 * store. A sheet rising from the bottom on phones, a panel floating next to
 * its trigger on wider screens. Long-press (or right-click) a stamp for its
 * details.
 */
export function StampPicker({ onPick, trigger, keepOpen }: StampPickerProps) {
  const { t } = useTranslation("stamps");
  const [open, setOpen] = useState(false);
  const wide = useMediaQuery("(min-width: 640px)");
  // Focus goes to the panel, not into its search box: on a phone that would raise the keyboard over the stamps.
  const popup = useRef<HTMLDivElement>(null);
  const panel = open && (
    <PickerPanel
      onPick={(picked) => {
        onPick(picked);
        if (!keepOpen) setOpen(false);
      }}
      onLeave={() => setOpen(false)}
    />
  );

  if (wide) {
    return (
      <Popover.Root open={open} onOpenChange={setOpen}>
        <Popover.Trigger render={trigger} />
        <Popover.Portal>
          <Popover.Positioner
            side="top"
            align="end"
            sideOffset={10}
            collisionPadding={12}
            className="z-50 outline-none"
          >
            <Popover.Popup
              ref={popup}
              initialFocus={popup}
              aria-label={t("picker.title")}
              data-testid="stamp-picker"
              className={cn(
                "sticker-lg flex h-[27rem] max-h-[var(--available-height)] w-[23.5rem] origin-[var(--transform-origin)] flex-col overflow-hidden rounded-[1.75rem] bg-surface outline-none",
                "transition-[transform,opacity] duration-150 data-[ending-style]:scale-95 data-[ending-style]:opacity-0 data-[starting-style]:scale-95 data-[starting-style]:opacity-0",
              )}
            >
              {panel}
            </Popover.Popup>
          </Popover.Positioner>
        </Popover.Portal>
      </Popover.Root>
    );
  }
  return (
    <BaseDialog.Root open={open} onOpenChange={setOpen}>
      <BaseDialog.Trigger render={trigger} />
      <BaseDialog.Portal>
        <BaseDialog.Backdrop className="fixed inset-0 z-50 bg-overlay transition-opacity duration-200 data-[ending-style]:opacity-0 data-[starting-style]:opacity-0" />
        <BaseDialog.Popup
          ref={popup}
          initialFocus={popup}
          data-testid="stamp-picker"
          className={cn(
            "sticker-lg fixed inset-x-0 bottom-0 z-50 flex h-[min(30rem,68dvh)] flex-col overflow-hidden rounded-t-[2rem] bg-surface pb-safe outline-none",
            "transition-transform duration-300 ease-out-soft data-[ending-style]:translate-y-full data-[starting-style]:translate-y-full",
          )}
        >
          <BaseDialog.Title className="sr-only">{t("picker.title")}</BaseDialog.Title>
          <span
            aria-hidden
            className="mx-auto mt-2 h-1.5 w-10 shrink-0 rounded-full bg-surface-3"
          />
          {panel}
        </BaseDialog.Popup>
      </BaseDialog.Portal>
    </BaseDialog.Root>
  );
}

function useMediaQuery(query: string): boolean {
  return useSyncExternalStore(
    (notify) => {
      const list = window.matchMedia(query);
      list.addEventListener("change", notify);
      return () => list.removeEventListener("change", notify);
    },
    () => window.matchMedia(query).matches,
    () => false,
  );
}

/** Tabs that are not packs. Pack tabs are keyed by the pack's ID. */
const SHELVES = ["recents", "favorites", "personal"] as const;

function PickerPanel({
  onPick,
  onLeave,
}: {
  onPick(stamp: common.StampRef): void;
  onLeave(): void;
}) {
  const { t } = useTranslation("stamps");
  // What was sent since the picker was last open belongs in "recently used".
  const library = useLibrary({ refetchOnMount: "always" });
  const [chosen, setChosen] = useState<string | null>(null);
  const [text, setText] = useState("");
  const [query, setQuery] = useState("");
  const [detail, setDetail] = useState<string>();
  const [making, setMaking] = useState(false);
  useEffect(() => {
    const timer = setTimeout(() => setQuery(text.trim()), 200);
    return () => clearTimeout(timer);
  }, [text]);

  const packs = useInstalledPacks(library.data);
  // Start where there is something to pick: recents, else the first pack.
  const tab =
    chosen ??
    (library.data && library.data.recents.length === 0 && packs[0] ? packs[0].id : "recents");
  const shelf = SHELVES.find((value) => value === tab);

  return (
    <>
      <div className="flex shrink-0 items-center gap-2 px-3 pt-3">
        <SearchField
          className="h-10 min-w-0 flex-1 px-3"
          value={text}
          onChange={setText}
          placeholder={t("picker.search")}
        />
        <Link
          to="/stamps"
          onClick={onLeave}
          aria-label={t("picker.store")}
          title={t("picker.store")}
          className="pressable sticker-sm grid size-10 shrink-0 place-items-center rounded-full bg-lime text-[#17122b]"
        >
          <Store className="size-5" />
        </Link>
      </div>

      {!query && (
        <div
          role="tablist"
          aria-label={t("picker.tabs")}
          className="scrollbar-none flex shrink-0 gap-1 overflow-x-auto border-b-2 border-line px-3 py-2"
        >
          <ShelfTab
            active={tab === "recents"}
            label={t("picker.recents")}
            onSelect={() => setChosen("recents")}
          >
            <Clock className="size-5" />
          </ShelfTab>
          <ShelfTab
            active={tab === "favorites"}
            label={t("picker.favorites")}
            onSelect={() => setChosen("favorites")}
          >
            <Heart className="size-5" />
          </ShelfTab>
          <ShelfTab
            active={tab === "personal"}
            label={t("picker.personal")}
            onSelect={() => setChosen("personal")}
          >
            <Scissors className="size-5" />
          </ShelfTab>
          {packs.map((pack) => (
            <ShelfTab
              key={pack.id}
              active={tab === pack.id}
              label={pack.name}
              onSelect={() => setChosen(pack.id)}
            >
              <PackCover pack={pack} className="size-8 rounded-lg" />
            </ShelfTab>
          ))}
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-2 py-2">
        {query ? (
          <SearchShelf query={query} onPick={onPick} onDetail={setDetail} />
        ) : library.isPending ? (
          <TileGrid>
            {Array.from({ length: 12 }, (_, index) => (
              <Skeleton key={index} className="aspect-square" />
            ))}
          </TileGrid>
        ) : shelf === "recents" ? (
          <Tiles
            stamps={library.data?.recents ?? []}
            onPick={onPick}
            onDetail={setDetail}
            empty={
              <EmptyState
                emoji="🕊️"
                title={t("picker.recentsEmpty")}
                body={packs.length === 0 ? t("picker.noPacksBody") : t("picker.recentsEmptyBody")}
              />
            }
          />
        ) : shelf === "favorites" ? (
          <PagedShelf
            kind="favorites"
            first={library.data?.favorites ?? []}
            more={library.data?.moreFavorites ?? false}
            onPick={onPick}
            onDetail={setDetail}
          />
        ) : shelf === "personal" ? (
          <PagedShelf
            kind="personal"
            first={library.data?.personal ?? []}
            more={library.data?.morePersonal ?? false}
            onPick={onPick}
            onDetail={setDetail}
            onMake={() => setMaking(true)}
          />
        ) : (
          <PackShelf packId={tab} onPick={onPick} onDetail={setDetail} />
        )}
      </div>

      <StampSheet stampId={detail} onClose={() => setDetail(undefined)} />
      <PersonalStampMaker open={making} onOpenChange={setMaking} />
    </>
  );
}

/** Installed packs for the tab strip: the library's first hundred, or all of them when there are more. */
function useInstalledPacks(library: stamp.GetLibraryResponse | undefined): stamp.PackSummary[] {
  const all = useQuery({
    queryKey: [...stampKeys.installed, "all"],
    queryFn: async () => {
      const packs: stamp.PackSummary[] = [];
      let cursor = "";
      do {
        const page = await libraryClient().listInstalledPacks({ page: { limit: 200, cursor } });
        packs.push(...page.packs);
        cursor = page.page?.nextCursor ?? "";
      } while (cursor);
      return packs;
    },
    enabled: Boolean(library?.moreInstalled),
  });
  return all.data ?? library?.installed ?? [];
}

function ShelfTab({
  active,
  label,
  onSelect,
  children,
}: {
  active: boolean;
  label: string;
  onSelect(): void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      aria-label={label}
      title={label}
      onClick={onSelect}
      className={cn(
        "grid size-11 shrink-0 place-items-center rounded-2xl transition-colors",
        active ? "sticker-sm bg-brand-soft text-brand" : "text-ink-soft hover:bg-surface-2",
      )}
    >
      {children}
    </button>
  );
}

function TileGrid({ children }: { children: ReactNode }) {
  return <div className="grid grid-cols-4 gap-1">{children}</div>;
}

/** How long a finger rests on a stamp before its details open instead of it being picked. */
const LONG_PRESS_MS = 450;

/** A stamp in the picker: a tap picks it, a long press or right-click opens its details. */
function PickTile({
  item,
  onPick,
  onDetail,
}: {
  item: stamp.Stamp;
  onPick(stamp: common.StampRef): void;
  onDetail(stampId: string): void;
}) {
  const { t } = useTranslation("stamps");
  const press = useRef<{ timer: ReturnType<typeof setTimeout>; x: number; y: number } | null>(null);
  const held = useRef(false);
  const ref = item.stampRef;
  if (!ref) return null;
  const release = () => {
    if (press.current) clearTimeout(press.current.timer);
    press.current = null;
  };
  return (
    <button
      type="button"
      data-testid="picker-stamp"
      aria-label={ref.name || t("stamp.unnamed")}
      className="relative rounded-2xl p-1.5 transition-transform duration-150 hover:bg-surface-2 active:scale-90"
      style={{ WebkitTouchCallout: "none" }}
      onClick={() => {
        // The press already opened the details.
        if (held.current) held.current = false;
        else onPick(ref);
      }}
      onContextMenu={(event) => {
        event.preventDefault();
        release();
        onDetail(ref.id);
      }}
      onPointerDown={(event) => {
        held.current = false;
        if (event.pointerType === "mouse") return;
        const timer = setTimeout(() => {
          held.current = true;
          press.current = null;
          onDetail(ref.id);
        }, LONG_PRESS_MS);
        press.current = { timer, x: event.clientX, y: event.clientY };
      }}
      onPointerMove={(event) => {
        // Scrolling the grid is not a press.
        if (
          press.current &&
          Math.hypot(event.clientX - press.current.x, event.clientY - press.current.y) > 8
        )
          release();
      }}
      onPointerUp={release}
      onPointerCancel={release}
      onPointerLeave={release}
    >
      <StampImage stamp={ref} image={item.image} className="block" />
      {item.favorited && (
        <span className="pointer-events-none absolute top-0.5 right-0.5 grid size-4 place-items-center rounded-full border-[1.5px] border-ink bg-pink text-white">
          <Heart className="size-2 fill-current" />
        </span>
      )}
    </button>
  );
}

function Tiles({
  stamps,
  onPick,
  onDetail,
  empty,
  before,
}: {
  stamps: stamp.Stamp[];
  onPick(stamp: common.StampRef): void;
  onDetail(stampId: string): void;
  empty: ReactNode;
  /** A tile shown before the stamps (the "make one" button). */
  before?: ReactNode;
}) {
  if (stamps.length === 0) return <>{empty}</>;
  return (
    <TileGrid>
      {before}
      {stamps.map((item) => (
        <PickTile key={item.stampRef?.id} item={item} onPick={onPick} onDetail={onDetail} />
      ))}
    </TileGrid>
  );
}

/** Favourites or personal stamps: what the library call returned, and the rest page by page when there is more. */
function PagedShelf({
  kind,
  first,
  more,
  onPick,
  onDetail,
  onMake,
}: {
  kind: "favorites" | "personal";
  first: stamp.Stamp[];
  more: boolean;
  onPick(stamp: common.StampRef): void;
  onDetail(stampId: string): void;
  onMake?(): void;
}) {
  const { t } = useTranslation("stamps");
  const rest = useInfiniteQuery({
    queryKey: [...(kind === "favorites" ? stampKeys.favorites : stampKeys.personal), "picker"],
    queryFn: async ({ pageParam }) => {
      const page = { limit: 60, cursor: pageParam };
      const response =
        kind === "favorites"
          ? await libraryClient().listFavoriteStamps({ page })
          : await libraryClient().listPersonalStamps({ page });
      return { stamps: response.stamps, next: response.page?.nextCursor ?? "" };
    },
    initialPageParam: "",
    getNextPageParam: (last) => last.next || undefined,
    enabled: more,
  });
  const stamps = rest.data ? rest.data.pages.flatMap((page) => page.stamps) : first;
  // The grid sits in its own scroll box, so the next page is asked for from its scroll position.
  const sentinel = useRef<HTMLDivElement>(null);
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = rest;
  useEffect(() => {
    const element = sentinel.current;
    if (!element || !hasNextPage || isFetchingNextPage) return;
    const observer = new IntersectionObserver(
      (entries) => entries.some((entry) => entry.isIntersecting) && fetchNextPage(),
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  const make = onMake && (
    <button
      type="button"
      onClick={onMake}
      aria-label={t("picker.make")}
      title={t("picker.make")}
      className="m-1.5 grid aspect-square place-items-center rounded-2xl border-2 border-dashed border-line text-ink-soft transition-colors hover:border-brand hover:text-brand"
    >
      <Plus className="size-6" />
    </button>
  );
  return (
    <>
      <Tiles
        stamps={stamps}
        onPick={onPick}
        onDetail={onDetail}
        before={make}
        empty={
          kind === "favorites" ? (
            <EmptyState
              emoji="💗"
              title={t("picker.favoritesEmpty")}
              body={t("picker.favoritesEmptyBody")}
            />
          ) : (
            <EmptyState
              emoji="✂️"
              title={t("picker.personalEmpty")}
              body={t("picker.personalEmptyBody")}
              action={
                <button
                  type="button"
                  onClick={onMake}
                  className="pressable sticker inline-flex h-10 items-center gap-1.5 rounded-full bg-lime px-4 text-sm font-bold text-[#17122b]"
                >
                  <Plus className="size-4" />
                  {t("picker.make")}
                </button>
              }
            />
          )
        }
      />
      {hasNextPage && <div ref={sentinel} className="h-8" />}
    </>
  );
}

function PackShelf({
  packId,
  onPick,
  onDetail,
}: {
  packId: string;
  onPick(stamp: common.StampRef): void;
  onDetail(stampId: string): void;
}) {
  const { t } = useTranslation("stamps");
  const { data: pack, isPending } = usePack(packId);
  if (isPending) {
    return (
      <TileGrid>
        {Array.from({ length: 8 }, (_, index) => (
          <Skeleton key={index} className="aspect-square" />
        ))}
      </TileGrid>
    );
  }
  return (
    <>
      {pack?.summary && (
        <p className="truncate px-2 pb-1 text-xs font-black text-ink-faint">{pack.summary.name}</p>
      )}
      <Tiles
        stamps={pack?.stamps.filter((item) => item.usable) ?? []}
        onPick={onPick}
        onDetail={onDetail}
        empty={<EmptyState emoji="🫙" title={t("picker.packEmpty")} />}
      />
    </>
  );
}

function SearchShelf({
  query,
  onPick,
  onDetail,
}: {
  query: string;
  onPick(stamp: common.StampRef): void;
  onDetail(stampId: string): void;
}) {
  const { t } = useTranslation("stamps");
  const { data, isPending } = useQuery({
    queryKey: stampKeys.librarySearch(query),
    queryFn: () => libraryClient().searchLibraryStamps({ query, limit: 60 }),
  });
  if (isPending) return null;
  return (
    <Tiles
      stamps={data?.stamps ?? []}
      onPick={onPick}
      onDetail={onDetail}
      empty={<EmptyState emoji="🔍" title={t("picker.searchEmpty", { query })} />}
    />
  );
}
