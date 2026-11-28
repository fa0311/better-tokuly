import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { live, video } from "@tokuly/api";
import { Gift, Search, Video, X } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { BalancePill } from "#/features/wallet";
import { cn } from "#/lib/cn";
import { realtime } from "#/lib/realtime";
import { useTranslation } from "#/lib/use-translation";
import { liveKeys, nextCursor, streamClient, useFriendStreams } from "./api";
import { CATEGORIES } from "./labels";
import { StreamCard, StreamCardSkeleton, StreamGrid } from "./parts";

const { StreamStatus } = live;
const { VideoCategory } = video;

export type BrowseTab = "now" | "upcoming";

function Shelf({
  title,
  streams,
  testId,
}: {
  title: string;
  streams: live.StreamSummary[];
  testId: string;
}) {
  if (streams.length === 0) return null;
  return (
    <section className="mb-6" data-testid={testId}>
      <h2 className="mb-2 text-lg font-black">{title}</h2>
      <div className="scrollbar-none -mx-4 flex gap-4 overflow-x-auto px-4 pb-1">
        {streams.map((stream) => (
          <StreamCard key={stream.id} stream={stream} className="w-64 shrink-0" />
        ))}
      </div>
    </section>
  );
}

function CategoryChips({
  value,
  onChange,
}: {
  value: video.VideoCategory;
  onChange(category: video.VideoCategory): void;
}) {
  const { t } = useTranslation("live");
  const chip = (category: video.VideoCategory, label: ReactNode) => (
    <button
      key={category}
      type="button"
      aria-pressed={value === category}
      onClick={() => onChange(category)}
      className={cn(
        "flex h-9 shrink-0 items-center gap-1 rounded-full border-2 px-3.5 text-sm font-bold",
        value === category
          ? "border-ink bg-ink text-surface"
          : "border-line bg-surface text-ink-soft hover:text-ink",
      )}
    >
      {label}
    </button>
  );
  return (
    <fieldset className="scrollbar-none -mx-4 flex gap-2 overflow-x-auto px-4 pb-1">
      <legend className="sr-only">{t("browse.categories")}</legend>
      {chip(VideoCategory.UNSPECIFIED, t("browse.allCategories"))}
      {CATEGORIES.map((category) =>
        chip(
          category.value,
          <>
            <span aria-hidden>{category.emoji}</span>
            {t(`category.${category.key}`)}
          </>,
        ),
      )}
    </fieldset>
  );
}

function StreamResults({
  query,
  empty,
}: {
  query: ReturnType<typeof useInfiniteQuery<{ streams: live.StreamSummary[] }>>;
  empty: ReactNode;
}) {
  const streams = query.data?.pages.flatMap((page) => page.streams) ?? [];
  if (query.isPending)
    return (
      <StreamGrid>
        {Array.from({ length: 6 }, (_, index) => (
          <StreamCardSkeleton key={index} />
        ))}
      </StreamGrid>
    );
  if (streams.length === 0) return empty;
  return (
    <>
      <StreamGrid>
        {streams.map((stream) => (
          <StreamCard key={stream.id} stream={stream} />
        ))}
      </StreamGrid>
      <LoadMore query={query} />
    </>
  );
}

function LiveNow() {
  const { t } = useTranslation("live");
  const [category, setCategory] = useState<video.VideoCategory>(VideoCategory.UNSPECIFIED);
  const [gameText, setGameText] = useState("");
  const [game, setGame] = useState("");
  const friends = useFriendStreams();
  const subscribed = useQuery({
    queryKey: liveKeys.subscribed,
    queryFn: async () =>
      (await streamClient().listSubscribedStreams({ page: { limit: 20 } })).streams,
    staleTime: 30_000,
  });
  const query = useInfiniteQuery({
    queryKey: liveKeys.liveNow(category, game),
    queryFn: ({ pageParam }) =>
      streamClient().listLiveStreams({ category, game, page: { cursor: pageParam, limit: 24 } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  });
  // A friend's stream is not repeated under the channels the viewer subscribes to.
  const friendIds = new Set(friends.data?.map((stream) => stream.id));
  return (
    <>
      <Shelf title={t("browse.friends")} streams={friends.data ?? []} testId="friend-streams" />
      <Shelf
        title={t("browse.subscribed")}
        streams={(subscribed.data ?? []).filter((stream) => !friendIds.has(stream.id))}
        testId="subscribed-streams"
      />
      <h2 className="mb-2 text-lg font-black">{t("browse.liveNow")}</h2>
      <CategoryChips value={category} onChange={setCategory} />
      <form
        className="mt-3 mb-4 flex h-11 items-center gap-2 rounded-full border-2 border-line bg-surface pr-1.5 pl-4 focus-within:border-brand"
        onSubmit={(event) => {
          event.preventDefault();
          setGame(gameText.trim());
        }}
      >
        <Search className="size-4 shrink-0 text-ink-faint" />
        <input
          value={gameText}
          onChange={(event) => setGameText(event.target.value)}
          placeholder={t("browse.gamePlaceholder")}
          aria-label={t("browse.gameLabel")}
          enterKeyHint="search"
          className="h-full min-w-0 flex-1 bg-transparent text-[15px] outline-none placeholder:text-ink-faint"
        />
        {(game || gameText) && (
          <button
            type="button"
            aria-label={t("browse.gameClear")}
            onClick={() => {
              setGame("");
              setGameText("");
            }}
            className="grid size-8 place-items-center rounded-full text-ink-soft hover:bg-surface-2"
          >
            <X className="size-4" />
          </button>
        )}
      </form>
      <StreamResults
        query={query}
        empty={
          <EmptyState
            emoji="📡"
            title={t(game ? "browse.emptyGameTitle" : "browse.emptyNowTitle", { game })}
            body={t("browse.emptyNowBody")}
            action={
              <Link
                to="/studio/live/new"
                className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-brand px-5 font-bold text-on-brand"
              >
                <Video className="size-4" />
                {t("browse.goLive")}
              </Link>
            }
          />
        }
      />
    </>
  );
}

function Upcoming() {
  const { t } = useTranslation("live");
  const [category, setCategory] = useState<video.VideoCategory>(VideoCategory.UNSPECIFIED);
  const friends = useFriendStreams();
  const query = useInfiniteQuery({
    queryKey: liveKeys.upcoming(category),
    queryFn: ({ pageParam }) =>
      streamClient().listUpcomingStreams({ category, page: { cursor: pageParam, limit: 24 } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  });
  return (
    <>
      <Shelf
        title={t("browse.friendsUpcoming")}
        streams={(friends.data ?? []).filter((stream) => stream.status === StreamStatus.UPCOMING)}
        testId="friend-upcoming"
      />
      <CategoryChips value={category} onChange={setCategory} />
      <div className="mt-4">
        <StreamResults
          query={query}
          empty={
            <EmptyState
              emoji="🗓️"
              title={t("browse.emptyUpcomingTitle")}
              body={t("browse.emptyUpcomingBody")}
            />
          }
        />
      </div>
    </>
  );
}

/** `/live`: what is on air — friends first — by genre and game, and what is scheduled. */
export function BrowseLiveScreen({
  tab,
  onTabChange,
}: {
  tab: BrowseTab;
  onTabChange(tab: BrowseTab): void;
}) {
  const { t } = useTranslation("live");
  const queryClient = useQueryClient();
  // Lists are read again when the connection returns from a gap: streams start and end all the time.
  useEffect(() => {
    const client = realtime();
    let welcomed = false;
    const off = client.onFrame((frame) => {
      if (frame.body.case !== "welcome") return;
      if (welcomed) void queryClient.invalidateQueries({ queryKey: liveKeys.lists });
      welcomed = true;
    });
    return off;
  }, [queryClient]);
  return (
    <Page
      title={t("browse.title")}
      wide
      actions={
        <>
          <Link
            to="/live/gifts"
            aria-label={t("giftHistory.title")}
            title={t("giftHistory.title")}
            className="grid size-9 shrink-0 place-items-center rounded-full hover:bg-surface-2"
          >
            <Gift className="size-5" />
          </Link>
          <BalancePill />
          <Link
            to="/studio/live"
            className="pressable sticker-sm inline-flex h-9 shrink-0 items-center gap-1.5 rounded-full bg-brand px-3.5 text-sm font-black text-on-brand"
          >
            <Video className="size-4" />
            <span className="max-sm:sr-only">{t("browse.goLive")}</span>
          </Link>
        </>
      }
    >
      <Tabs
        className="mb-4"
        items={[
          { value: "now", label: t("browse.tabs.now") },
          { value: "upcoming", label: t("browse.tabs.upcoming") },
        ]}
        value={tab}
        onChange={onTabChange}
      />
      {tab === "now" ? <LiveNow /> : <Upcoming />}
    </Page>
  );
}
