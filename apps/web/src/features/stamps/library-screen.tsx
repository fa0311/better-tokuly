import {
  type InfiniteData,
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { stamp, toDate } from "@tokuly/api";
import { contextKey, stampKeys, stampQueries } from "@tokuly/core/stamps";
import { GripVertical, MoreHorizontal, Plus, Smile, Trash2, X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { LoadMore } from "#/components/ui/load-more";
import { Menu } from "#/components/ui/menu";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Sortable } from "#/components/ui/sortable";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { libraryClient, useInstallPack, useUninstallPack } from "./api";
import { PersonalStampMaker } from "./editor/stamp-maker";
import { LIBRARY_TABS, type LibraryTab } from "./library-search";
import { PackRow, SearchField, StampGrid, StampTile } from "./parts";
import { StampPicker } from "./picker";
import { StampImage } from "./stamp-image";
import { StampSheet } from "./stamp-sheet";

type OpenStamp = (stampId: string | undefined) => void;

/** "マイスタンプ": everything the user holds — packs, their own stamps, favourites, history. */
export function LibraryScreen({ tab }: { tab: LibraryTab }) {
  const { t } = useTranslation("stamps");
  const navigate = useNavigate();
  const [text, setText] = useState("");
  const [query, setQuery] = useState("");
  const [openStamp, setOpenStamp] = useState<string>();
  useEffect(() => {
    const timer = setTimeout(() => setQuery(text.trim()), 250);
    return () => clearTimeout(timer);
  }, [text]);

  return (
    <Page
      title={t("library.title")}
      back="/stamps"
      wide
      actions={
        // The picker as chats and comments show it: a quick way through the whole library.
        <StampPicker
          trigger={
            <Button size="sm" variant="secondary" icon={<Smile className="size-4" />}>
              {t("library.picker")}
            </Button>
          }
          onPick={(picked) => setOpenStamp(picked.id)}
        />
      }
    >
      <div className="flex flex-col gap-3">
        <SearchField value={text} onChange={setText} placeholder={t("library.searchPlaceholder")} />
        {query ? (
          <SearchResults query={query} onOpen={setOpenStamp} />
        ) : (
          <>
            <Tabs<LibraryTab>
              className="self-start"
              value={tab}
              onChange={(next) =>
                navigate({
                  to: "/stamps/library",
                  search: { tab: next === "packs" ? undefined : next },
                  replace: true,
                })
              }
              items={LIBRARY_TABS.map((value) => ({ value, label: t(`library.tabs.${value}`) }))}
            />
            {tab === "packs" && <InstalledPacks />}
            {tab === "personal" && <PersonalStamps onOpen={setOpenStamp} />}
            {tab === "favorites" && <Favorites onOpen={setOpenStamp} />}
            {tab === "recents" && <Recents onOpen={setOpenStamp} />}
            {tab === "purchased" && <Purchased />}
            {tab === "log" && <UsageLog onOpen={setOpenStamp} />}
          </>
        )}
      </div>
      <StampSheet stampId={openStamp} onClose={() => setOpenStamp(undefined)} />
    </Page>
  );
}

function GridSkeleton() {
  return (
    <StampGrid>
      {Array.from({ length: 12 }, (_, index) => (
        <Skeleton key={index} className="aspect-square" />
      ))}
    </StampGrid>
  );
}

function RowsSkeleton() {
  return (
    <div className="flex flex-col gap-2">
      {[0, 1, 2, 3].map((index) => (
        <Skeleton key={index} className="h-[4.5rem]" />
      ))}
    </div>
  );
}

function StoreLink() {
  const { t } = useTranslation("stamps");
  return (
    <Link
      to="/stamps"
      className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand"
    >
      {t("library.toStore")}
    </Link>
  );
}

// ───────────────────────────── Search ─────────────────────────────

function SearchResults({ query, onOpen }: { query: string; onOpen: OpenStamp }) {
  const { t } = useTranslation("stamps");
  const { data, isPending } = useQuery({
    queryKey: stampKeys.librarySearch(query),
    queryFn: () => libraryClient().searchLibraryStamps({ query, limit: 60 }),
  });
  if (isPending) return <GridSkeleton />;
  if (!data?.stamps.length)
    return (
      <EmptyState
        emoji="🔍"
        title={t("library.searchEmpty", { query })}
        body={t("library.searchEmptyBody")}
      />
    );
  return (
    <StampGrid>
      {data.stamps.map((item) => (
        <StampTile
          key={item.stampRef?.id}
          item={item}
          onPress={(pressed) => onOpen(pressed.stampRef?.id)}
        />
      ))}
    </StampGrid>
  );
}

// ───────────────────────────── Installed packs ─────────────────────────────

type InstalledPages = InfiniteData<stamp.ListInstalledPacksResponse, string>;

function InstalledPacks() {
  const { t } = useTranslation("stamps");
  const queryClient = useQueryClient();
  const installed = useInfiniteQuery(stampQueries.installed(libraryClient));
  const packs = installed.data?.pages.flatMap((page) => page.packs) ?? [];
  const uninstall = useUninstallPack();
  const [confirming, setConfirming] = useState<stamp.PackSummary | null>(null);

  const reorder = useMutation({
    mutationFn: (ordered: stamp.PackSummary[]) =>
      libraryClient().reorderInstalledPacks({ packIds: ordered.map((pack) => pack.id) }),
    // The list shows the new order at once: everything loaded so far moves into the first page.
    onMutate: (ordered) =>
      queryClient.setQueryData<InstalledPages>(
        stampKeys.installed,
        (data) =>
          data && {
            ...data,
            pages: data.pages.map((page, index) => ({
              ...page,
              packs: index === 0 ? ordered : [],
            })),
          },
      ),
    onError: (error) => {
      toast.error(errorMessage(error, { namespace: "stamps" }));
      return queryClient.invalidateQueries({ queryKey: stampKeys.installed });
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: stampKeys.libraryHome }),
  });

  // Uninstalling a pack that left the store is final unless it was bought or is the user's own.
  const remove = (pack: stamp.PackSummary) => {
    const comesBack =
      pack.status === stamp.PackStatus.PUBLISHED || pack.viewer?.owned || pack.viewer?.canManage;
    if (comesBack) uninstall.mutate(pack.id);
    else setConfirming(pack);
  };

  if (installed.isPending) return <RowsSkeleton />;
  if (packs.length === 0)
    return (
      <EmptyState
        emoji="🧺"
        title={t("library.packsEmpty")}
        body={t("library.packsEmptyBody")}
        action={<StoreLink />}
      />
    );

  return (
    <>
      <p className="px-1 text-xs font-bold text-ink-faint">{t("library.packsHint")}</p>
      <Sortable
        items={packs}
        getKey={(pack) => pack.id}
        onReorder={(ordered) => reorder.mutate(ordered)}
        className="flex flex-col gap-2 md:grid md:grid-cols-2 md:gap-3"
      >
        {(pack, { grip, dragging }) => (
          <div
            className={cn(
              "sticker-sm flex items-center rounded-2xl bg-surface pl-1",
              dragging && "bg-brand-soft",
            )}
            data-testid="installed-pack"
          >
            <button
              type="button"
              {...grip}
              aria-label={t("library.reorder", { name: pack.name })}
              className="grid h-14 w-8 shrink-0 place-items-center rounded-xl text-ink-faint hover:text-ink"
            >
              <GripVertical className="size-5" />
            </button>
            <PackRow
              pack={pack}
              className="min-w-0 flex-1 px-1"
              trailing={
                <Menu
                  trigger={
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      aria-label={t("library.packMenu", { name: pack.name })}
                    >
                      <MoreHorizontal className="size-5" />
                    </Button>
                  }
                  actions={[
                    {
                      label: t("pack.uninstall"),
                      icon: <Trash2 />,
                      danger: true,
                      onSelect: () => remove(pack),
                    },
                  ]}
                />
              }
            />
          </div>
        )}
      </Sortable>
      <LoadMore query={installed} />
      <ConfirmDialog
        open={confirming !== null}
        onOpenChange={(open) => !open && setConfirming(null)}
        title={t("library.uninstallGoneTitle")}
        description={t("library.uninstallGoneBody", { name: confirming?.name ?? "" })}
        confirmLabel={t("pack.uninstall")}
        danger
        loading={uninstall.isPending}
        onConfirm={() =>
          confirming && uninstall.mutate(confirming.id, { onSettled: () => setConfirming(null) })
        }
      />
    </>
  );
}

// ───────────────────────────── Personal stamps ─────────────────────────────

/** Reordering needs the whole list, so this loads every page (the server caps personal stamps at a few hundred). */
async function allPersonalStamps(): Promise<stamp.Stamp[]> {
  const stamps: stamp.Stamp[] = [];
  let cursor = "";
  do {
    const page = await libraryClient().listPersonalStamps({ page: { limit: 100, cursor } });
    stamps.push(...page.stamps);
    cursor = page.page?.nextCursor ?? "";
  } while (cursor);
  return stamps;
}

function PersonalStamps({ onOpen }: { onOpen: OpenStamp }) {
  const { t } = useTranslation("stamps");
  const queryClient = useQueryClient();
  const [making, setMaking] = useState(false);
  const personal = useQuery({ queryKey: stampKeys.personal, queryFn: allPersonalStamps });

  const reorder = useMutation({
    mutationFn: (ordered: stamp.Stamp[]) =>
      libraryClient().reorderPersonalStamps({ stampIds: ordered.map((item) => item.stampRef!.id) }),
    onMutate: (ordered) => queryClient.setQueryData(stampKeys.personal, ordered),
    onError: (error) => {
      toast.error(errorMessage(error, { namespace: "stamps" }));
      return queryClient.invalidateQueries({ queryKey: stampKeys.personal });
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: stampKeys.libraryHome }),
  });

  const stamps = personal.data ?? [];
  return (
    <>
      <div className="flex items-center justify-between gap-3">
        <p className="min-w-0 px-1 text-xs font-bold text-ink-faint">
          {stamps.length > 1 ? t("library.personalHint") : t("library.personalLead")}
        </p>
        <Button
          size="sm"
          className="shrink-0"
          icon={<Plus className="size-4" />}
          onClick={() => setMaking(true)}
        >
          {t("library.make")}
        </Button>
      </div>
      {personal.isPending ? (
        <GridSkeleton />
      ) : stamps.length === 0 ? (
        <EmptyState
          emoji="✂️"
          title={t("library.personalEmpty")}
          body={t("library.personalEmptyBody")}
          action={
            <Button
              variant="lime"
              icon={<Plus className="size-4" />}
              onClick={() => setMaking(true)}
            >
              {t("library.makeFirst")}
            </Button>
          }
        />
      ) : (
        <Sortable
          items={stamps}
          getKey={(item) => item.stampRef?.id ?? ""}
          onReorder={(ordered) => reorder.mutate(ordered)}
          grab="press"
          className="grid grid-cols-4 gap-1 sm:grid-cols-5 md:grid-cols-6 lg:grid-cols-8"
        >
          {(item, { grip, dragging }) => (
            <div {...grip} className={cn("rounded-2xl", dragging && "bg-brand-soft")}>
              <StampTile item={item} onPress={(pressed) => onOpen(pressed.stampRef?.id)} />
            </div>
          )}
        </Sortable>
      )}
      <PersonalStampMaker open={making} onOpenChange={setMaking} />
    </>
  );
}

// ───────────────────────────── Favourites ─────────────────────────────

function Favorites({ onOpen }: { onOpen: OpenStamp }) {
  const { t } = useTranslation("stamps");
  const favorites = useInfiniteQuery({
    queryKey: stampKeys.favorites,
    queryFn: ({ pageParam }) =>
      libraryClient().listFavoriteStamps({ page: { limit: 60, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const stamps = favorites.data?.pages.flatMap((page) => page.stamps) ?? [];
  if (favorites.isPending) return <GridSkeleton />;
  if (stamps.length === 0)
    return (
      <EmptyState
        emoji="💗"
        title={t("library.favoritesEmpty")}
        body={t("library.favoritesEmptyBody")}
      />
    );
  return (
    <>
      <StampGrid>
        {stamps.map((item) => (
          <StampTile
            key={item.stampRef?.id}
            item={item}
            onPress={(pressed) => onOpen(pressed.stampRef?.id)}
          />
        ))}
      </StampGrid>
      <LoadMore query={favorites} />
    </>
  );
}

// ───────────────────────────── Recently used ─────────────────────────────

type RecentOrder = "last" | "most";

function Recents({ onOpen }: { onOpen: OpenStamp }) {
  const { t } = useTranslation("stamps");
  const queryClient = useQueryClient();
  const [order, setOrder] = useState<RecentOrder>("last");
  const [tidying, setTidying] = useState(false);
  const [confirmClear, setConfirmClear] = useState(false);
  const value = order === "most" ? stamp.RecentOrder.MOST_USED : stamp.RecentOrder.LAST_USED;
  const recents = useQuery({
    queryKey: stampKeys.recents(value),
    queryFn: () => libraryClient().listRecentStamps({ order: value, limit: 64 }),
  });
  const stamps = recents.data?.stamps ?? [];

  const refresh = () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: ["stamp", "library", "recents"] }),
      queryClient.invalidateQueries({ queryKey: stampKeys.libraryHome }),
    ]);
  const forget = useMutation({
    mutationFn: (stampId: string) => libraryClient().clearRecentStamps({ stampId }),
    // The tile disappears at once, from both orders.
    onMutate: (stampId) =>
      queryClient.setQueriesData<stamp.ListRecentStampsResponse>(
        { queryKey: ["stamp", "library", "recents"] },
        (data) =>
          data && { ...data, stamps: data.stamps.filter((item) => item.stampRef?.id !== stampId) },
      ),
    onError: (error) => toast.error(errorMessage(error, { namespace: "stamps" })),
    onSettled: refresh,
  });
  const clear = useMutation({
    mutationFn: () => libraryClient().clearRecentStamps({}),
    onSuccess: () => {
      setConfirmClear(false);
      setTidying(false);
      toast.success(t("toast.recentsCleared"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "stamps" })),
    onSettled: refresh,
  });

  return (
    <>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <Tabs<RecentOrder>
          value={order}
          onChange={setOrder}
          items={[
            { value: "last", label: t("library.recentOrder.last") },
            { value: "most", label: t("library.recentOrder.most") },
          ]}
        />
        {stamps.length > 0 && (
          <div className="flex gap-1">
            <Button
              size="sm"
              variant={tidying ? "secondary" : "ghost"}
              aria-pressed={tidying}
              onClick={() => setTidying((on) => !on)}
            >
              {tidying ? t("common:actions.done") : t("library.tidy")}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              className="text-danger"
              onClick={() => setConfirmClear(true)}
            >
              {t("library.clearRecents")}
            </Button>
          </div>
        )}
      </div>
      {recents.isPending ? (
        <GridSkeleton />
      ) : stamps.length === 0 ? (
        <EmptyState
          emoji="🕊️"
          title={t("library.recentsEmpty")}
          body={t("library.recentsEmptyBody")}
        />
      ) : (
        <StampGrid>
          {stamps.map((item) => (
            <StampTile
              key={item.stampRef?.id}
              item={item}
              onPress={(pressed) => onOpen(pressed.stampRef?.id)}
              className={tidying ? "animate-wiggle" : undefined}
            >
              {tidying && (
                <button
                  type="button"
                  aria-label={t("library.forget", { name: item.stampRef?.name ?? "" })}
                  onClick={() => item.stampRef && forget.mutate(item.stampRef.id)}
                  className="absolute -top-0.5 -left-0.5 grid size-6 place-items-center rounded-full border-2 border-ink bg-danger text-white"
                >
                  <X className="size-3.5" strokeWidth={3} />
                </button>
              )}
            </StampTile>
          ))}
        </StampGrid>
      )}
      <ConfirmDialog
        open={confirmClear}
        onOpenChange={setConfirmClear}
        title={t("library.clearRecentsTitle")}
        description={t("library.clearRecentsBody")}
        confirmLabel={t("library.clearRecents")}
        danger
        loading={clear.isPending}
        onConfirm={() => clear.mutate()}
      />
    </>
  );
}

// ───────────────────────────── Purchases ─────────────────────────────

function Purchased() {
  const { t } = useTranslation("stamps");
  const install = useInstallPack();
  const purchased = useInfiniteQuery({
    queryKey: stampKeys.purchased,
    queryFn: ({ pageParam }) =>
      libraryClient().listPurchasedPacks({ page: { limit: 50, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const packs = purchased.data?.pages.flatMap((page) => page.packs) ?? [];
  if (purchased.isPending) return <RowsSkeleton />;
  if (packs.length === 0)
    return (
      <EmptyState
        emoji="🪙"
        title={t("library.purchasedEmpty")}
        body={t("library.purchasedEmptyBody")}
        action={<StoreLink />}
      />
    );
  return (
    <>
      <div className="flex flex-col gap-2 md:grid md:grid-cols-2 md:gap-3">
        {packs.map((pack) => (
          <PackRow
            key={pack.id}
            pack={pack}
            className="sticker-sm bg-surface"
            trailing={
              pack.viewer?.installed ? (
                <Chip className="shrink-0 text-success">{t("state.installed")}</Chip>
              ) : (
                <Button
                  size="sm"
                  variant="lime"
                  className="shrink-0"
                  loading={install.isPending && install.variables === pack.id}
                  onClick={() => install.mutate(pack.id)}
                >
                  {t("library.addBack")}
                </Button>
              )
            }
          />
        ))}
      </div>
      <LoadMore query={purchased} />
    </>
  );
}

// ───────────────────────────── Usage log ─────────────────────────────

/** "今日" / "昨日" / a date with its weekday. */
function dayLabel(date: Date, language: string, today: string, yesterday: string): string {
  const startOfDay = (value: Date) =>
    new Date(value.getFullYear(), value.getMonth(), value.getDate()).getTime();
  const days = Math.round((startOfDay(new Date()) - startOfDay(date)) / 86_400_000);
  if (days === 0) return today;
  if (days === 1) return yesterday;
  return date.toLocaleDateString(language, {
    year: date.getFullYear() === new Date().getFullYear() ? undefined : "numeric",
    month: "long",
    day: "numeric",
    weekday: "short",
  });
}

function UsageLog({ onOpen }: { onOpen: OpenStamp }) {
  const { t, i18n } = useTranslation("stamps");
  const log = useInfiniteQuery({
    queryKey: stampKeys.usageLog,
    queryFn: ({ pageParam }) =>
      libraryClient().listUsageLog({ page: { limit: 50, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const entries = log.data?.pages.flatMap((page) => page.entries) ?? [];
  if (log.isPending) return <RowsSkeleton />;
  if (entries.length === 0)
    return <EmptyState emoji="📮" title={t("library.logEmpty")} body={t("library.logEmptyBody")} />;

  // Entries arrive newest first; a heading starts each new day.
  const days: { label: string; entries: stamp.UsageEntry[] }[] = [];
  for (const entry of entries) {
    const sent = toDate(entry.sentAt);
    const label = sent
      ? dayLabel(sent, i18n.language, t("library.today"), t("library.yesterday"))
      : "";
    const day = days[days.length - 1];
    if (day && day.label === label) day.entries.push(entry);
    else days.push({ label, entries: [entry] });
  }

  return (
    <>
      <div className="flex flex-col gap-4">
        {days.map((day) => (
          <section key={day.label}>
            <h2 className="px-2 pb-1 text-sm font-black text-ink-soft">{day.label}</h2>
            <ul className="sticker-sm grid rounded-2xl bg-surface p-1 md:grid-cols-2">
              {day.entries.map((entry) => {
                const ref = entry.stamp?.stampRef;
                if (!ref) return null;
                return (
                  <li key={entry.id}>
                    <button
                      type="button"
                      onClick={() => onOpen(ref.id)}
                      className="flex w-full items-center gap-3 rounded-xl px-2 py-1.5 text-left transition-colors hover:bg-surface-2"
                    >
                      <StampImage stamp={ref} image={entry.stamp?.image} size={48} play="never" />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate font-bold">
                          {ref.name || t("stamp.unnamed")}
                        </span>
                        <span className="block text-xs text-ink-soft">
                          {t("library.sentTo", { where: t(contextKey(entry.context)) })}
                        </span>
                      </span>
                      <time
                        className="shrink-0 text-xs font-bold text-ink-faint tabular-nums"
                        dateTime={toDate(entry.sentAt)?.toISOString()}
                      >
                        {toDate(entry.sentAt)?.toLocaleTimeString(i18n.language, {
                          hour: "2-digit",
                          minute: "2-digit",
                        })}
                      </time>
                    </button>
                  </li>
                );
              })}
            </ul>
          </section>
        ))}
      </div>
      <LoadMore query={log} showEnd />
    </>
  );
}
