import { useInfiniteQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useCanGoBack, useNavigate, useRouter } from "@tanstack/react-router";
import type { shorts } from "@tokuly/api";
import { type FeedContext, playbackSource } from "@tokuly/core/shorts";
import {
  Bookmark,
  ChevronLeft,
  Clapperboard,
  Hash,
  LayoutGrid,
  Plus,
  Volume2,
  VolumeX,
} from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { Menu } from "#/components/ui/menu";
import { Spinner } from "#/components/ui/spinner";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useHideTabBar } from "#/stores/shell";
import { useShort } from "../api";
import { type FeedEntry, listQuery } from "../lists";
import { ShortPager } from "./pager";
import { setFeedMuted, useFeedSound } from "./use-short-video";
import { type FeedWindow, moveWindow, type PageInWindow } from "./window";

export const FEED_TABS = ["forYou", "following", "friends"] as const;
export type FeedTab = (typeof FEED_TABS)[number];

export interface FeedSearch {
  tab?: Exclude<FeedTab, "forYou">;
}

export function validateFeedSearch(search: Record<string, unknown>): FeedSearch {
  return search.tab === "following" || search.tab === "friends" ? { tab: search.tab } : {};
}

/** Where the viewer was in each feed, so coming back from a hashtag page lands on the same short. */
const positions = new Map<string, number>();

/** Each feed's feedWindow, so coming back to a feed whose front pages were let go keeps its positions. */
const windows = new Map<string, FeedWindow>();

/** A link that looks like the lime button, on the dark message slides. */
const pill =
  "pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-lime px-5 text-[15px] font-bold text-[#17122b]";

const barButton =
  "pointer-events-auto grid size-10 shrink-0 place-items-center rounded-full bg-black/35 text-white backdrop-blur-sm outline-none focus-visible:ring-[3px] focus-visible:ring-white";

/** The sound switch of the feed: one setting for every short, remembered. */
function MuteButton() {
  const { t } = useTranslation("shorts");
  const muted = useFeedSound((sound) => sound.muted || sound.blocked);
  return (
    <button
      type="button"
      className={barButton}
      aria-label={muted ? t("player.unmute") : t("player.mute")}
      aria-pressed={muted}
      data-testid="short-mute"
      onClick={() => setFeedMuted(!muted)}
    >
      {muted ? <VolumeX className="size-5" /> : <Volume2 className="size-5" />}
    </button>
  );
}

/** The frame of a swipe feed: black to the edges on a phone, a 9:16 stage next to the rail on wide screens. */
function Stage({ bar, children }: { bar: ReactNode; children: ReactNode }) {
  useHideTabBar();
  return (
    <div className="flex h-full justify-center bg-black md:bg-bg md:py-4">
      <div className="relative h-full w-full md:aspect-[9/16] md:w-auto md:max-w-[calc(100%-10rem)]">
        {children}
        <div className="pointer-events-none absolute inset-x-0 top-0 z-10 bg-gradient-to-b from-black/55 to-transparent px-3 pt-safe pb-6 md:rounded-t-[2rem]">
          <div className="flex items-center gap-1.5">{bar}</div>
          <div className="mt-2 flex justify-end">
            <MuteButton />
          </div>
        </div>
      </div>
    </div>
  );
}

/** `/shorts`: the three feeds, one swipe apart. */
export function FeedScreen({
  search,
  onSearchChange,
}: {
  search: FeedSearch;
  onSearchChange(next: FeedSearch): void;
}) {
  const { t } = useTranslation("shorts");
  const navigate = useNavigate();
  const tab: FeedTab = search.tab ?? "forYou";
  return (
    <Stage
      bar={
        <>
          <Link to="/watch" className={barButton} aria-label={t("bar.back")}>
            <ChevronLeft className="size-6" />
          </Link>
          <div role="tablist" className="pointer-events-auto flex min-w-0 flex-1 justify-center">
            {FEED_TABS.map((value) => (
              <button
                key={value}
                type="button"
                role="tab"
                aria-selected={tab === value}
                onClick={() => onSearchChange(value === "forYou" ? {} : { tab: value })}
                className={cn(
                  "relative h-10 shrink-0 px-2 text-[15px] font-black outline-none [text-shadow:0_1px_3px_rgb(0_0_0/0.7)] focus-visible:underline",
                  tab === value ? "text-white" : "text-white/65",
                )}
              >
                {t(`tabs.${value}`)}
                {tab === value && (
                  <span className="absolute inset-x-3 bottom-1 h-[3px] rounded-full bg-white" />
                )}
              </button>
            ))}
          </div>
          <Menu
            trigger={
              <button type="button" className={barButton} aria-label={t("bar.more")}>
                <LayoutGrid className="size-5" />
              </button>
            }
            actions={[
              {
                label: t("bar.post"),
                icon: <Plus />,
                onSelect: () => void navigate({ to: "/shorts/new" }),
              },
              {
                label: t("bar.saved"),
                icon: <Bookmark />,
                onSelect: () => void navigate({ to: "/shorts/saved" }),
              },
              {
                label: t("bar.mine"),
                icon: <Clapperboard />,
                onSelect: () => void navigate({ to: "/shorts/studio" }),
              },
              {
                label: t("bar.tags"),
                icon: <Hash />,
                onSelect: () => void navigate({ to: "/shorts/tags" }),
              },
            ]}
          />
        </>
      }
    >
      <Feed key={tab} context={{ kind: tab }} />
    </Stage>
  );
}

/** `/shorts/<id>`: the swipe feed opened on one short, inside the list it was tapped in. */
export function ShortScreen({ shortId, context }: { shortId: string; context: FeedContext }) {
  const { t } = useTranslation("shorts");
  const router = useRouter();
  const canGoBack = useCanGoBack();
  const navigate = useNavigate();
  const title =
    context.kind === "channel"
      ? `@${context.key}`
      : context.kind === "hashtag"
        ? `#${context.key}`
        : t(`bar.title.${context.kind}`);
  return (
    <Stage
      bar={
        <>
          <button
            type="button"
            className={barButton}
            aria-label={t("bar.back")}
            onClick={() => (canGoBack ? router.history.back() : void navigate({ to: "/shorts" }))}
          >
            <ChevronLeft className="size-6" />
          </button>
          <h1 className="min-w-0 flex-1 truncate text-center text-[15px] font-black text-white [text-shadow:0_1px_3px_rgb(0_0_0/0.7)]">
            {title}
          </h1>
          <Link to="/shorts" className={barButton} aria-label={t("bar.feed")}>
            <LayoutGrid className="size-5" />
          </Link>
        </>
      }
    >
      <Feed key={shortId} context={context} startId={shortId} />
    </Stage>
  );
}

function Message({
  emoji,
  title,
  body,
  children,
}: {
  emoji: string;
  title: string;
  body?: string;
  children?: ReactNode;
}) {
  return (
    <div className="flex size-full flex-col items-center justify-center gap-2 bg-[#17122b] px-8 text-center text-white md:rounded-[2rem]">
      <div className="animate-float text-6xl" aria-hidden>
        {emoji}
      </div>
      <p className="mt-2 text-lg font-black">{title}</p>
      {body && <p className="max-w-xs text-sm leading-relaxed text-white/75">{body}</p>}
      {children && <div className="mt-3 flex flex-col items-center gap-2">{children}</div>}
    </div>
  );
}

const Loading = () => (
  <div className="grid size-full place-items-center bg-[#17122b] md:rounded-[2rem]">
    <Spinner className="size-9 text-white/80" />
  </div>
);

/** Loads a feed's list (and the short to open on) and hands both to the pager once it knows where to start. */
function Feed({ context, startId }: { context: FeedContext; startId?: string }) {
  const { t } = useTranslation("shorts");
  const canSkipSeen = context.kind === "following" || context.kind === "friends";
  const [skipSeen, setSkipSeen] = useState(canSkipSeen);
  const options = listQuery(context, { skipSeen });
  const query = useInfiniteQuery(options);
  const start = useShort(startId);
  const entries = query.data?.pages.flatMap((page) => page.entries) ?? [];
  const positionKey = JSON.stringify(options.queryKey);

  if (startId && start.isPending) return <Loading />;
  if (startId && !start.data) {
    return (
      <Message emoji="🫥" title={t("feed.gone")} body={t("feed.goneBody")}>
        <Link to="/shorts" className={pill}>
          {t("feed.toForYou")}
        </Link>
      </Message>
    );
  }
  // Opened by its link, the short plays at once and the feed behind it may still be loading.
  if (query.isPending && !(start.data && context.kind === "link")) return <Loading />;
  if (query.isError && entries.length === 0 && !start.data) {
    return (
      <Message emoji="😵‍💫" title={errorMessage(query.error, { namespace: "shorts" })}>
        <Button variant="secondary" onClick={() => void query.refetch()}>
          {t("feed.retry")}
        </Button>
      </Message>
    );
  }
  return (
    <ReadyFeed
      key={positionKey}
      context={context}
      start={start.data}
      entries={entries}
      positionKey={positionKey}
      pages={(query.data?.pages ?? []).map((page, index) => ({
        param: query.data?.pageParams[index] as string,
        length: page.entries.length,
      }))}
      hasMore={query.hasNextPage}
      loadingMore={query.isFetchingNextPage || query.isPending}
      loadMore={() => void query.fetchNextPage()}
      hasEarlier={query.hasPreviousPage}
      loadingEarlier={query.isFetchingPreviousPage}
      loadEarlier={() => void query.fetchPreviousPage()}
      skipSeen={skipSeen}
      onShowSeen={canSkipSeen ? () => setSkipSeen(false) : undefined}
    />
  );
}

function ReadyFeed({
  context,
  start,
  entries: listEntries,
  positionKey,
  pages,
  hasMore,
  loadingMore,
  loadMore,
  hasEarlier,
  loadingEarlier,
  loadEarlier,
  skipSeen,
  onShowSeen,
}: {
  context: FeedContext;
  start?: shorts.Short;
  entries: FeedEntry[];
  positionKey: string;
  /** The loaded pages, by the cursor each was asked with, and how many entries each holds now. */
  pages: PageInWindow[];
  hasMore: boolean;
  loadingMore: boolean;
  loadMore(): void;
  hasEarlier: boolean;
  loadingEarlier: boolean;
  loadEarlier(): void;
  skipSeen: boolean;
  onShowSeen?: () => void;
}) {
  const { t } = useTranslation("shorts");
  const queryClient = useQueryClient();
  // Where the loaded pages sit in the whole feed: they move on as pages far behind are let go.
  const [feedWindow, setFeedWindow] = useState<FeedWindow>(() => {
    const kept = windows.get(positionKey);
    return kept && kept.pages[0]?.param === pages[0]?.param ? kept : { pages, offset: 0 };
  });
  const moved = moveWindow(feedWindow, pages);
  if (moved !== feedWindow) setFeedWindow(moved);
  useEffect(() => {
    windows.set(positionKey, feedWindow);
  }, [positionKey, feedWindow]);

  // Decided once, when the feed opens: a list that moved under the viewer would be a different feed.
  const [arrangement] = useState(() => {
    if (!start) {
      const remembered = positions.get(positionKey) ?? 0;
      const first = feedWindow.offset;
      return {
        pin: false,
        index: Math.min(Math.max(remembered, first), first + Math.max(0, listEntries.length - 1)),
      };
    }
    const found =
      context.kind === "link" ? -1 : listEntries.findIndex((entry) => entry.short.id === start.id);
    // In the list it was tapped in, the short keeps its place (and what comes before it); otherwise it leads.
    return found >= 0 ? { pin: false, index: found } : { pin: true, index: 0 };
  });
  // A short opened by its link leads the feed, until the pages after it are let go too.
  const pinned = arrangement.pin && start && feedWindow.offset === 0;
  const offset =
    arrangement.pin && feedWindow.offset > 0 ? feedWindow.offset + 1 : feedWindow.offset;
  const entries =
    pinned && start
      ? [
          ...(start.viewer?.notInterested ? [] : [{ short: start }]),
          ...listEntries.filter((entry) => entry.short.id !== start.id),
        ]
      : listEntries;
  const [current, setCurrent] = useState(arrangement.index);

  useEffect(() => {
    if (!start) positions.set(positionKey, current);
  }, [start, positionKey, current]);

  // The next page is asked for a few shorts before the end, so the swipe never waits; the same going back.
  const nearEnd = current >= offset + entries.length - 3;
  useEffect(() => {
    if (nearEnd && hasMore && !loadingMore) loadMore();
  }, [nearEnd, hasMore, loadingMore, loadMore]);
  const nearStart = current < offset + 2;
  useEffect(() => {
    if (nearStart && hasEarlier && !loadingEarlier && !loadingMore) loadEarlier();
  }, [nearStart, hasEarlier, loadingEarlier, loadingMore, loadEarlier]);

  const toForYou = (
    <Link to="/shorts" className={pill}>
      {t("feed.toForYou")}
    </Link>
  );
  const post = (
    <Link to="/shorts/new" className={pill}>
      <Plus className="size-5" />
      {t("feed.post")}
    </Link>
  );
  const end =
    hasMore || loadingMore ? (
      <Loading />
    ) : skipSeen && onShowSeen ? (
      <Message emoji="✨" title={t("feed.caughtUp")} body={t("feed.caughtUpBody")}>
        <Button variant="lime" onClick={onShowSeen}>
          {t("feed.showSeen")}
        </Button>
        {toForYou}
      </Message>
    ) : entries.length === 0 && offset === 0 ? (
      context.kind === "following" ? (
        <Message emoji="📡" title={t("feed.emptyFollowing")} body={t("feed.emptyFollowingBody")}>
          {toForYou}
        </Message>
      ) : context.kind === "friends" ? (
        <Message emoji="🫶" title={t("feed.emptyFriends")} body={t("feed.emptyFriendsBody")}>
          {post}
          {toForYou}
        </Message>
      ) : (
        <Message emoji="🎬" title={t("feed.empty")} body={t("feed.emptyBody")}>
          {post}
        </Message>
      )
    ) : context.kind === "forYou" || context.kind === "link" ? (
      <Message emoji="🎉" title={t("feed.end")} body={t("feed.endBody")}>
        <Button
          variant="lime"
          onClick={() => {
            positions.delete(positionKey);
            void queryClient.resetQueries({ queryKey: listQuery({ kind: "forYou" }).queryKey });
          }}
        >
          {t("feed.again")}
        </Button>
      </Message>
    ) : (
      <Message emoji="🎉" title={t("feed.end")}>
        {toForYou}
      </Message>
    );

  return (
    <ShortPager
      entries={entries}
      offset={offset}
      source={playbackSource(context)}
      startIndex={arrangement.index}
      onIndexChange={setCurrent}
      end={end}
    />
  );
}
