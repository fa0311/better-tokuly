import { useInfiniteQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { video } from "@tokuly/api";
import {
  BarChart3,
  Eye,
  EyeOff,
  MessageSquareText,
  MoreVertical,
  Pencil,
  Play,
  Plus,
  Star,
  Trash2,
  Upload,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import { toast } from "sonner";
import { CountBadge } from "#/components/ui/badge";
import { ConfirmDialog } from "#/components/ui/confirm";
import { LoadMore } from "#/components/ui/load-more";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { useHeldCount } from "#/features/comments";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import {
  channelClient,
  nextCursor,
  patchVideoEverywhere,
  removeVideoEverywhere,
  videoClient,
  videoKeys,
} from "../api";
import { formatCount } from "../format";
import { ChannelAvatar, ChannelName, Thumbnail } from "../video-card";
import { ChannelPicker, selectStudioChannel, useStudioChannel, VideoStateChips } from "./shared";

const { VideoStatus, ProcessingState } = video;

type Filter = "all" | "published" | "scheduled" | "draft";
const FILTER_STATUS: Record<Filter, video.VideoStatus> = {
  all: VideoStatus.UNSPECIFIED,
  published: VideoStatus.PUBLISHED,
  scheduled: VideoStatus.SCHEDULED,
  draft: VideoStatus.DRAFT,
};

/**
 * `/studio`: the creator's home. Without a channel it explains the three
 * steps to a first video; with one it shows the channel, the shortcuts
 * (upload, analytics, comments) and every video with its state.
 */
export function StudioHome() {
  const { t } = useTranslation("video");
  const { query, channels, channel, maxChannels } = useStudioChannel();
  const held = useHeldCount(channels.length > 0);

  if (query.isPending) {
    return (
      <Page title={t("studio.title")} back="/watch" wide>
        <Skeleton className="h-28 rounded-card" />
        <Skeleton className="mt-4 h-64 rounded-card" />
      </Page>
    );
  }
  if (query.isError) {
    return (
      <Page title={t("studio.title")} back="/watch" wide>
        <EmptyState emoji="😵‍💫" title={errorMessage(query.error, { namespace: "video" })} />
      </Page>
    );
  }
  const summary = channel?.summary;
  if (!channel || !summary) return <Welcome />;

  return (
    <Page title={t("studio.title")} back="/watch" wide>
      <ChannelPicker channels={channels} selectedId={summary.id} onSelect={selectStudioChannel} />

      <section
        className="sticker mt-3 flex flex-col gap-4 rounded-card bg-surface p-4 sm:flex-row sm:items-center"
        data-testid="studio-channel"
      >
        <ChannelAvatar channel={summary} size="lg" outlined />
        <div className="min-w-0 flex-1">
          <p className="flex flex-wrap items-center gap-2 text-xl font-black">
            <ChannelName channel={summary} className="max-w-full" />
            {channel.viewer?.isDefault && (
              <span className="inline-flex h-6 items-center gap-1 rounded-full bg-sun px-2.5 text-xs font-black text-[#17122b]">
                <Star className="size-3.5 fill-current" />
                {t("studio.defaultChannel")}
              </span>
            )}
          </p>
          <p className="flex flex-wrap gap-x-3 text-sm font-bold text-ink-soft">
            <span>@{summary.handle}</span>
            <span>
              {t("channel.subscribers", {
                count: Number(summary.subscriberCount),
                formatted: formatCount(summary.subscriberCount),
              })}
            </span>
            <span>
              {t("card.views", {
                count: Number(channel.viewCount),
                formatted: formatCount(channel.viewCount),
              })}
            </span>
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Link
            to="/c/$handle"
            params={{ handle: summary.handle }}
            className="pressable sticker-sm inline-flex h-10 items-center gap-1.5 rounded-full bg-surface px-4 text-sm font-bold"
          >
            <Eye className="size-4" />
            {t("studio.viewChannel")}
          </Link>
          <Link
            to="/studio/channels/$channelId"
            params={{ channelId: summary.id }}
            className="pressable sticker-sm inline-flex h-10 items-center gap-1.5 rounded-full bg-surface px-4 text-sm font-bold"
          >
            <Pencil className="size-4" />
            {t("actions.edit")}
          </Link>
        </div>
      </section>

      <nav
        className="mt-4 grid grid-cols-2 gap-3 lg:grid-cols-4"
        aria-label={t("studio.shortcuts")}
      >
        <Shortcut
          to="/studio/upload"
          tint="bg-brand text-on-brand"
          icon={<Upload className="size-6" />}
          label={t("studio.upload.action")}
        />
        <Shortcut
          to="/studio/analytics"
          tint="bg-sky text-[#17122b]"
          icon={<BarChart3 className="size-6" />}
          label={t("studio.analytics.title")}
        />
        <Shortcut
          to="/settings/comments"
          tint="bg-pink text-white"
          icon={<MessageSquareText className="size-6" />}
          label={t("studio.comments")}
          badge={held.data}
        />
        {channels.length < maxChannels && (
          <Shortcut
            to="/studio/channels/new"
            tint="bg-lime text-[#17122b]"
            icon={<Plus className="size-6" />}
            label={t("studio.addChannel")}
          />
        )}
      </nav>

      <VideoList key={summary.id} channelId={summary.id} />
    </Page>
  );
}

function Shortcut({
  to,
  tint,
  icon,
  label,
  badge,
}: {
  to: "/studio/upload" | "/studio/analytics" | "/settings/comments" | "/studio/channels/new";
  tint: string;
  icon: ReactNode;
  label: string;
  badge?: number;
}) {
  return (
    <Link
      to={to}
      className="pressable sticker flex h-24 flex-col justify-between rounded-card bg-surface p-3.5"
    >
      <span className="flex items-start justify-between">
        <span className={`grid size-10 place-items-center rounded-2xl border-2 border-ink ${tint}`}>
          {icon}
        </span>
        <CountBadge count={badge ?? 0} />
      </span>
      <span className="truncate text-sm font-black">{label}</span>
    </Link>
  );
}

/** What someone without a channel sees: the three steps to a first video. */
function Welcome() {
  const { t } = useTranslation("video");
  const steps = ["studio.welcome.step1", "studio.welcome.step2", "studio.welcome.step3"] as const;
  return (
    <Page title={t("studio.title")} back="/watch">
      <section className="sticker rounded-card bg-surface p-6 text-center">
        <p className="animate-float text-6xl" aria-hidden>
          🎬
        </p>
        <h2 className="mt-3 text-2xl font-black">{t("studio.welcome.title")}</h2>
        <p className="mx-auto mt-2 max-w-sm text-sm leading-relaxed text-ink-soft">
          {t("studio.welcome.lead")}
        </p>
        <ol className="mx-auto mt-6 flex max-w-sm flex-col gap-3 text-left">
          {steps.map((key, index) => (
            <li key={key} className="flex items-center gap-3 rounded-2xl bg-surface-2 p-3">
              <span className="grid size-9 shrink-0 place-items-center rounded-full border-2 border-ink bg-lime font-black text-[#17122b]">
                {index + 1}
              </span>
              <span className="font-bold">{t(key)}</span>
            </li>
          ))}
        </ol>
        <Link
          to="/studio/channels/new"
          className="pressable sticker mt-6 inline-flex h-14 items-center gap-2 rounded-full bg-brand px-7 text-lg font-bold text-on-brand"
        >
          <Plus className="size-5" />
          {t("studio.welcome.action")}
        </Link>
      </section>
    </Page>
  );
}

/** Every video of a channel, whatever its state, newest first. */
function VideoList({ channelId }: { channelId: string }) {
  const { t } = useTranslation("video");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [filter, setFilter] = useState<Filter>("all");
  const [deleting, setDeleting] = useState<video.VideoSummary | null>(null);
  const list = useInfiniteQuery({
    queryKey: videoKeys.studioVideos(channelId, FILTER_STATUS[filter]),
    queryFn: ({ pageParam }) =>
      videoClient().listStudioVideos({
        channelId,
        status: FILTER_STATUS[filter],
        page: { limit: 30, cursor: pageParam },
      }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  });
  const videos = list.data?.pages.flatMap((page) => page.videos) ?? [];
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: "video" }));
  /** A state change moves the video between the filtered lists: read them again. */
  const refresh = () => void queryClient.invalidateQueries({ queryKey: videoKeys.studio });

  const publish = useMutation({
    mutationFn: (summary: video.VideoSummary) =>
      videoClient().publishVideo({ videoId: summary.id }),
    onSuccess: ({ video: updated }) => {
      if (updated?.summary)
        patchVideoEverywhere(queryClient, updated.summary.id, () => updated.summary ?? null);
      refresh();
      toast.success(
        updated?.summary?.status === VideoStatus.PUBLISHED
          ? t("studio.toast.published")
          : t("studio.toast.publishWhenReady"),
      );
    },
    onError,
  });
  const unpublish = useMutation({
    mutationFn: (summary: video.VideoSummary) =>
      videoClient().unpublishVideo({ videoId: summary.id }),
    onSuccess: ({ video: updated }) => {
      if (updated?.summary)
        patchVideoEverywhere(queryClient, updated.summary.id, () => updated.summary ?? null);
      refresh();
      toast.success(t("studio.toast.unpublished"));
    },
    onError,
  });
  const remove = useMutation({
    mutationFn: (summary: video.VideoSummary) => videoClient().deleteVideo({ videoId: summary.id }),
    onSuccess: (_response, summary) => {
      setDeleting(null);
      removeVideoEverywhere(queryClient, summary.id);
      void queryClient.invalidateQueries({ queryKey: videoKeys.channels });
      toast.success(t("studio.toast.deleted"));
    },
    onError: (error) => {
      setDeleting(null);
      onError(error);
    },
  });

  const actionsFor = (summary: video.VideoSummary): MenuAction[] => {
    const id = summary.shortId;
    const failed = summary.processing === ProcessingState.FAILED;
    return [
      {
        label: t("actions.edit"),
        icon: <Pencil />,
        onSelect: () => void navigate({ to: "/studio/videos/$videoId", params: { videoId: id } }),
      },
      {
        label: t("studio.videos.watch"),
        icon: <Play />,
        onSelect: () =>
          void navigate({ to: "/watch/$videoId", params: { videoId: id }, search: {} }),
      },
      {
        label: t("studio.analytics.title"),
        icon: <BarChart3 />,
        onSelect: () =>
          void navigate({
            to: "/studio/videos/$videoId",
            params: { videoId: id },
            search: { tab: "analytics" },
          }),
      },
      ...(summary.status === VideoStatus.DRAFT
        ? [
            {
              label: t("studio.publish.now"),
              icon: <Eye />,
              disabled: failed,
              onSelect: () => publish.mutate(summary),
            },
          ]
        : [
            {
              label: t("studio.publish.unpublish"),
              icon: <EyeOff />,
              onSelect: () => unpublish.mutate(summary),
            },
          ]),
      {
        label: t("actions.delete"),
        icon: <Trash2 />,
        danger: true,
        onSelect: () => setDeleting(summary),
      },
    ];
  };

  return (
    <section className="mt-8" aria-label={t("studio.videos.title")}>
      <div className="flex flex-wrap items-center gap-3">
        <h2 className="text-xl font-black">{t("studio.videos.title")}</h2>
        <span className="flex-1" />
        <Tabs<Filter>
          value={filter}
          onChange={setFilter}
          items={[
            { value: "all", label: t("studio.videos.all") },
            { value: "published", label: t("studio.state.published") },
            { value: "scheduled", label: t("studio.state.scheduled") },
            { value: "draft", label: t("studio.state.draft") },
          ]}
        />
      </div>
      <div className="mt-4">
        {list.isPending ? (
          <div className="flex flex-col gap-3">
            {[0, 1, 2].map((row) => (
              <Skeleton key={row} className="h-24 rounded-card" />
            ))}
          </div>
        ) : list.isError ? (
          <EmptyState emoji="😵‍💫" title={errorMessage(list.error, { namespace: "video" })} />
        ) : videos.length === 0 ? (
          filter === "all" ? (
            <EmptyState
              emoji="📹"
              title={t("studio.videos.empty")}
              body={t("studio.videos.emptyBody")}
              action={
                <Link
                  to="/studio/upload"
                  className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-brand px-5 font-bold text-on-brand"
                >
                  <Upload className="size-4" />
                  {t("studio.upload.action")}
                </Link>
              }
            />
          ) : (
            <EmptyState emoji="🔍" title={t("studio.videos.emptyFiltered")} />
          )
        ) : (
          <ul className="flex flex-col gap-3">
            {videos.map((summary) => (
              <li
                key={summary.id}
                data-testid="studio-video"
                className="sticker flex items-start gap-3 rounded-card bg-surface p-3"
              >
                <Link
                  to="/studio/videos/$videoId"
                  params={{ videoId: summary.shortId }}
                  className="w-32 shrink-0 sm:w-40"
                  tabIndex={-1}
                  aria-hidden
                >
                  <Thumbnail video={summary} sizes={160} />
                </Link>
                <div className="min-w-0 flex-1">
                  <Link
                    to="/studio/videos/$videoId"
                    params={{ videoId: summary.shortId }}
                    className="line-clamp-2 leading-snug font-black break-words hover:text-brand"
                  >
                    {summary.title}
                  </Link>
                  <VideoStateChips summary={summary} className="mt-1.5" />
                  <p className="mt-1.5 flex flex-wrap gap-x-3 text-xs font-bold text-ink-soft">
                    <span>
                      {t("card.views", {
                        count: Number(summary.viewCount),
                        formatted: formatCount(summary.viewCount),
                      })}
                    </span>
                    <span>
                      {t("studio.videos.likes", { formatted: formatCount(summary.likeCount) })}
                    </span>
                    <span>
                      {t("studio.videos.comments", {
                        formatted: formatCount(summary.commentCount),
                      })}
                    </span>
                  </p>
                </div>
                <Menu
                  trigger={
                    <button
                      type="button"
                      aria-label={t("card.menu", { title: summary.title })}
                      className="grid size-9 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-2 hover:text-ink"
                    >
                      <MoreVertical className="size-5" />
                    </button>
                  }
                  actions={actionsFor(summary)}
                />
              </li>
            ))}
            <LoadMore query={list} />
          </ul>
        )}
      </div>
      <ConfirmDialog
        open={deleting !== null}
        onOpenChange={(open) => !open && setDeleting(null)}
        title={t("studio.confirmDelete", { title: deleting?.title ?? "" })}
        description={t("studio.confirmDeleteBody")}
        confirmLabel={t("actions.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => deleting && remove.mutate(deleting)}
      />
    </section>
  );
}

/** Makes a channel the posting identity used when none is chosen. */
export function useSetDefaultChannel() {
  const queryClient = useQueryClient();
  const { t } = useTranslation("video");
  return useMutation({
    mutationFn: (channelId: string) => channelClient().setDefaultChannel({ channelId }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: videoKeys.myChannels });
      toast.success(t("studio.toast.defaultSet"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "video" })),
  });
}
