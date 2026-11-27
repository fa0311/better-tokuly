import { useInfiniteQuery, useMutation, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { video } from "@tokuly/api";
import {
  CATEGORIES,
  type Dismissal,
  FEED_SOURCES,
  type Feed,
  type FeedSearch,
  feedKey,
  feedVideos,
  loadFeedPage,
} from "@tokuly/core/video";
import { Ban, Clapperboard, History, ListVideo, ThumbsDown, Undo2, Users } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import type { MenuAction } from "#/components/ui/menu";
import { EmptyState, Page } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { channelClient, feedClient, nextCursor, videoKeys } from "./api";
import { ChannelRow } from "./channel/channel-row";
import { ChannelAvatar, VideoCard, VideoCardSkeleton, VideoGrid } from "./video-card";

const { VideoCategory } = video;

/**
 * `/watch`: the three feeds — recommendations, the channels the viewer
 * subscribes to, and what is trending — with genre chips over them.
 */
export function FeedsScreen({
  search,
  onSearchChange,
}: {
  search: FeedSearch;
  onSearchChange(search: FeedSearch): void;
}) {
  const { t } = useTranslation("video");
  const feed = search.feed ?? "home";
  const category = search.category ?? VideoCategory.UNSPECIFIED;
  const [dismissed, setDismissed] = useState<Record<string, Dismissal>>({});

  const query = useInfiniteQuery({
    // Trending is filtered by the server; the other two feeds are filtered here, from what is loaded.
    queryKey: feedKey(feed, category),
    queryFn: ({ pageParam }) => loadFeedPage(feedClient, feed, category, pageParam),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  });

  const mark = useMutation({
    mutationFn: async ({
      target,
      undo,
    }: {
      target: { case: "videoId" | "channelId"; value: string };
      undo: boolean;
    }) => {
      if (undo) await feedClient().undoNotInterested({ target });
      else await feedClient().markNotInterested({ target });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "video" })),
  });
  const dismiss = (summary: video.VideoSummary, kind: Dismissal["kind"]) => {
    const channelId = summary.channel?.id ?? "";
    setDismissed((current) => ({
      ...current,
      [summary.id]: kind === "video" ? { kind } : { kind, channelId },
    }));
    mark.mutate({
      target:
        kind === "video"
          ? { case: "videoId", value: summary.id }
          : { case: "channelId", value: channelId },
      undo: false,
    });
  };
  const restore = (summary: video.VideoSummary) => {
    const entry = dismissed[summary.id];
    if (!entry) return;
    setDismissed(({ [summary.id]: _restored, ...rest }) => rest);
    mark.mutate({
      target:
        entry.kind === "video"
          ? { case: "videoId", value: summary.id }
          : { case: "channelId", value: entry.channelId },
      undo: true,
    });
  };

  const videos = feedVideos(query.data?.pages ?? [], feed, category, dismissed);

  const setFeed = (next: Feed) =>
    onSearchChange({
      ...(next === "home" ? {} : { feed: next }),
      ...(search.category ? { category: search.category } : {}),
    });
  const setCategory = (next: video.VideoCategory) =>
    onSearchChange({
      ...(search.feed ? { feed: search.feed } : {}),
      ...(next ? { category: next } : {}),
    });

  return (
    <Page
      wide
      title={t("feeds.title")}
      actions={
        <nav className="flex items-center gap-0.5" aria-label={t("feeds.shortcuts")}>
          <HeaderLink
            to="/watch/history"
            label={t("history.title")}
            icon={<History className="size-5" />}
          />
          <HeaderLink
            to="/watch/playlists"
            label={t("playlist.libraryTitle")}
            icon={<ListVideo className="size-5" />}
          />
          <HeaderLink
            to="/watch/channels"
            label={t("subscriptions.title")}
            icon={<Users className="size-5" />}
          />
          <HeaderLink
            to="/studio"
            label={t("studio.title")}
            icon={<Clapperboard className="size-5" />}
          />
        </nav>
      }
    >
      <Tabs<Feed>
        value={feed}
        onChange={setFeed}
        items={[
          { value: "home", label: t("feeds.home") },
          { value: "subscriptions", label: t("feeds.subscriptions") },
          { value: "trending", label: t("feeds.trending") },
        ]}
      />
      <fieldset className="scrollbar-none -mx-4 mt-3 flex min-w-0 gap-2 overflow-x-auto px-4 pb-1">
        <legend className="sr-only">{t("feeds.categories")}</legend>
        <CategoryChip
          active={category === VideoCategory.UNSPECIFIED}
          onClick={() => setCategory(VideoCategory.UNSPECIFIED)}
        >
          {t("feeds.allCategories")}
        </CategoryChip>
        {CATEGORIES.map((entry) => (
          <CategoryChip
            key={entry.value}
            active={category === entry.value}
            onClick={() => setCategory(entry.value)}
          >
            <span aria-hidden>{entry.emoji}</span> {t(`category.${entry.key}`)}
          </CategoryChip>
        ))}
      </fieldset>

      {feed === "subscriptions" && <SubscribedStrip />}

      <div className="mt-5">
        {query.isPending ? (
          <VideoGrid>
            {Array.from({ length: 8 }, (_, index) => (
              <VideoCardSkeleton key={index} />
            ))}
          </VideoGrid>
        ) : query.isError ? (
          <EmptyState
            emoji="😵‍💫"
            title={errorMessage(query.error, { namespace: "video" })}
            action={<Button onClick={() => void query.refetch()}>{t("actions.retry")}</Button>}
          />
        ) : videos.length === 0 && !query.hasNextPage ? (
          <FeedEmpty feed={feed} filtered={category !== VideoCategory.UNSPECIFIED} />
        ) : (
          <>
            <VideoGrid>
              {videos.map((summary) => {
                const entry = dismissed[summary.id];
                if (entry)
                  return (
                    <Dismissed
                      key={summary.id}
                      kind={entry.kind}
                      channelName={summary.channel?.name ?? ""}
                      onUndo={() => restore(summary)}
                    />
                  );
                const actions: MenuAction[] =
                  feed === "home"
                    ? [
                        {
                          label: t("feeds.notInterested"),
                          icon: <ThumbsDown />,
                          onSelect: () => dismiss(summary, "video"),
                        },
                        ...(summary.channel
                          ? [
                              {
                                label: t("feeds.hideChannel"),
                                icon: <Ban />,
                                onSelect: () => dismiss(summary, "channel"),
                              },
                            ]
                          : []),
                      ]
                    : [];
                return (
                  <VideoCard
                    key={summary.id}
                    video={summary}
                    source={FEED_SOURCES[feed]}
                    actions={actions}
                  />
                );
              })}
            </VideoGrid>
            <LoadMore query={query} showEnd={videos.length > 12} />
          </>
        )}
      </div>
    </Page>
  );
}

function HeaderLink({
  to,
  label,
  icon,
}: {
  to: "/watch/history" | "/watch/playlists" | "/watch/channels" | "/studio";
  label: string;
  icon: ReactNode;
}) {
  return (
    <Link
      to={to}
      aria-label={label}
      title={label}
      className="grid size-10 place-items-center rounded-full text-ink-soft hover:bg-surface-2 hover:text-ink"
    >
      {icon}
    </Link>
  );
}

function CategoryChip({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick(): void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "h-9 shrink-0 rounded-full border-2 px-3.5 text-sm font-bold transition-colors",
        active
          ? "border-ink bg-ink text-bg"
          : "border-line bg-surface text-ink-soft hover:border-ink-faint hover:text-ink",
      )}
    >
      {children}
    </button>
  );
}

/** What stands where a card was after "not interested": a word of confirmation and the way back. */
function Dismissed({
  kind,
  channelName,
  onUndo,
}: {
  kind: Dismissal["kind"];
  channelName: string;
  onUndo(): void;
}) {
  const { t } = useTranslation("video");
  return (
    <div
      data-testid="video-dismissed"
      className="flex aspect-video flex-col items-center justify-center gap-3 rounded-2xl border-2 border-dashed border-line p-4 text-center"
    >
      <p className="text-sm font-bold text-ink-soft">
        {kind === "video"
          ? t("feeds.dismissedVideo")
          : t("feeds.dismissedChannel", { name: channelName })}
      </p>
      <Button size="sm" variant="secondary" icon={<Undo2 className="size-4" />} onClick={onUndo}>
        {t("feeds.undo")}
      </Button>
    </div>
  );
}

/** The channels the viewer subscribes to, as a row of faces above their feed. */
function SubscribedStrip() {
  const { t } = useTranslation("video");
  const subscriptions = useQuery({
    queryKey: [...videoKeys.subscriptions, "strip"],
    queryFn: () => channelClient().listSubscriptions({ page: { limit: 30 } }),
  });
  const channels =
    subscriptions.data?.subscriptions.flatMap((subscription) =>
      subscription.channel ? [subscription.channel] : [],
    ) ?? [];
  if (channels.length === 0) return null;
  return (
    <div className="scrollbar-none -mx-4 mt-4 flex items-start gap-3 overflow-x-auto px-4">
      {channels.map((channel) => (
        <Link
          key={channel.id}
          to="/c/$handle"
          params={{ handle: channel.handle }}
          className="flex w-16 shrink-0 flex-col items-center gap-1 text-center"
        >
          <ChannelAvatar channel={channel} size="md" outlined />
          <span className="w-full truncate text-[11px] font-bold text-ink-soft">
            {channel.name}
          </span>
        </Link>
      ))}
      <Link
        to="/watch/channels"
        className="grid h-11 shrink-0 place-items-center self-start rounded-full bg-surface-2 px-4 text-sm font-bold text-ink-soft hover:text-ink"
      >
        {t("feeds.allChannels")}
      </Link>
    </div>
  );
}

function FeedEmpty({ feed, filtered }: { feed: Feed; filtered: boolean }) {
  const { t } = useTranslation("video");
  if (filtered)
    return (
      <EmptyState emoji="🔍" title={t("feeds.emptyCategory")} body={t("feeds.emptyCategoryBody")} />
    );
  if (feed === "subscriptions") {
    return (
      <>
        <EmptyState
          emoji="📺"
          title={t("feeds.emptySubscriptions")}
          body={t("feeds.emptySubscriptionsBody")}
        />
        <RecommendedChannels />
      </>
    );
  }
  return (
    <EmptyState
      emoji="🎬"
      title={feed === "trending" ? t("feeds.emptyTrending") : t("feeds.emptyHome")}
      body={t("feeds.emptyHomeBody")}
      action={
        <Link
          to="/studio"
          className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-brand px-5 font-bold text-on-brand"
        >
          <Clapperboard className="size-4" />
          {t("feeds.goStudio")}
        </Link>
      }
    />
  );
}

/** Popular channels the viewer does not subscribe to yet. */
export function RecommendedChannels({ limit = 12 }: { limit?: number }) {
  const { t } = useTranslation("video");
  const recommended = useQuery({
    queryKey: [...videoKeys.recommendedChannels, limit],
    queryFn: () => channelClient().listRecommendedChannels({ limit }),
  });
  const channels = recommended.data?.channels ?? [];
  if (channels.length === 0) return null;
  return (
    <section className="mx-auto mt-2 max-w-2xl">
      <h2 className="px-1 text-lg font-black">{t("subscriptions.recommended")}</h2>
      <div className="sticker mt-3 rounded-card bg-surface p-2">
        {channels.map((channel) => (
          <ChannelRow key={channel.id} channel={channel} subscribed={false} />
        ))}
      </div>
    </section>
  );
}
