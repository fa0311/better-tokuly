import { useQueryClient } from "@tanstack/react-query";
import { common, toMillis, video } from "@tokuly/api";
import {
  audienceKey,
  blockKind,
  formatCount,
  imageUrl,
  playlistTitle,
  startPosition,
  videoKeys,
} from "@tokuly/core/video";
import { useEventListener } from "expo";
import { Image } from "expo-image";
import { router } from "expo-router";
import { useVideoPlayer, VideoView } from "expo-video";
import {
  Clock3,
  ListPlus,
  MessageCircle,
  Pencil,
  Share2,
  ThumbsDown,
  ThumbsUp,
} from "lucide-react-native";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Pressable,
  ScrollView,
  type ScrollViewInstance,
  Share,
  Text,
  useWindowDimensions,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Button } from "#/components/button";
import { EmptyState, Spinner } from "#/components/page";
import { webScreenHref } from "#/features/web/handoff";
import { mediaUrl, webLink } from "#/lib/config";
import { useTheme } from "#/lib/theme";
import { CommentsSheet } from "./comments-sheet";
import { usePlaylistQueue, useRate, useRelated, useSaveTo, useVideo } from "./data";
import { Chapters, Description } from "./description";
import { usePlaybackReport } from "./playback";
import { openSaveSheet } from "./save-sheet";
import { SubscribeButton } from "./subscribe-button";
import { ChannelAvatar, openChannel, sourceOf, VideoCard, videoMeta } from "./video-card";

const { Rating, VideoStatus, TrafficSource } = video;

/** What a watch route carries: `/watch/<id>?t=90&list=<playlist>&via=<traffic source>`. */
export interface WatchParams {
  t?: string;
  list?: string;
  /** Where the viewer came from (`video.TrafficSource`), set by the app's own links. */
  via?: string;
}

/** Seconds the "up next" card waits before it plays the next video by itself. */
const UP_NEXT_SECONDS = 8;

/** Opens the studio (or another creator tool) on the web screen. */
function openOnWeb(path: string): void {
  router.push(webScreenHref(path) as never);
}

/**
 * A video: the player on top (HLS, native controls, full screen and
 * picture-in-picture), then what it is and who made it, the things to do
 * with it, the chapters, the playlist it plays in, what to watch next, and
 * its comments.
 */
export function WatchScreen({ videoId, params }: { videoId: string; params: WatchParams }) {
  const { t } = useTranslation("video");
  const query = useVideo(videoId);
  if (query.isPending) return <Spinner />;
  const item = query.data;
  if (!item?.summary)
    return (
      <View testID="watch-not-found" className="flex-1 items-center justify-center bg-bg">
        <EmptyState emoji="🫥" title={t("watch.notFound")} body={t("watch.notFoundBody")} />
        <Button onPress={() => router.replace("/watch")}>{t("watch.backToFeeds")}</Button>
      </View>
    );
  return <Watch key={item.summary.id} videoId={videoId} item={item} params={params} />;
}

function Watch({
  videoId,
  item,
  params,
}: {
  videoId: string;
  item: video.Video;
  params: WatchParams;
}) {
  const { t } = useTranslation("video");
  const queryClient = useQueryClient();
  const { width } = useWindowDimensions();
  const summary = item.summary as video.VideoSummary;
  const channel = summary.channel;
  const isOwner = item.viewer?.isOwner ?? false;
  const scroller = useRef<ScrollViewInstance>(null);
  const [time, setTime] = useState(0);
  const [ended, setEnded] = useState(false);
  const [comments, setComments] = useState(false);
  const [source] = useState(() => (params.list ? TrafficSource.PLAYLIST : sourceOf(params.via)));

  const uri = mediaUrl(item.playback?.hlsUrl || item.playback?.mp4Url);
  const poster = mediaUrl(imageUrl(item.playback?.poster ?? summary.thumbnail, width));
  const player = useVideoPlayer(
    uri
      ? {
          uri,
          ...(item.playback?.hlsUrl ? { contentType: "hls" as const } : {}),
          metadata: { title: summary.title, artist: channel?.name, artwork: poster },
        }
      : null,
    (created) => {
      created.timeUpdateEventInterval = 1;
      // Picture-in-picture (and the lock screen's controls) keep playing outside the app.
      created.staysActiveInBackground = true;
      created.showNowPlayingNotification = true;
      created.currentTime = startPosition(params.t, summary);
      created.play();
    },
  );
  usePlaybackReport({
    videoId: item.playback ? summary.id : undefined,
    player,
    source,
    onViewCount: (viewCount) =>
      queryClient.setQueryData<video.Video | null>(videoKeys.video(videoId), (current) =>
        current?.summary && current.summary.viewCount !== viewCount
          ? { ...current, summary: { ...current.summary, viewCount } }
          : current,
      ),
  });
  useEventListener(player, "timeUpdate", ({ currentTime }) => setTime(Math.floor(currentTime)));
  useEventListener(player, "playToEnd", () => setEnded(true));
  useEventListener(player, "playingChange", ({ isPlaying }) => {
    if (isPlaying) setEnded(false);
  });

  const related = useRelated(summary.id, true);
  const queue = usePlaylistQueue(params.list, summary.id);
  const next = queue.next ?? related.data?.[0];
  const playNext = () => {
    if (!next) return;
    router.replace({
      pathname: "/watch/[videoId]",
      params: {
        videoId: next.shortId || next.id,
        ...(queue.next && params.list
          ? { list: params.list }
          : { via: String(TrafficSource.RELATED) }),
      },
    });
  };
  const seek = (seconds: number) => {
    player.currentTime = seconds;
    player.play();
    scroller.current?.scrollTo({ y: 0, animated: true });
  };

  const audience = audienceKey(summary.visibility, item.allowLink);
  return (
    <SafeAreaView edges={["bottom"]} className="flex-1 bg-bg" testID="watch-screen">
      <View className="aspect-video w-full bg-black">
        {item.playback ? (
          <VideoView
            testID="video-player"
            player={player}
            style={{ flex: 1 }}
            contentFit="contain"
            nativeControls
            fullscreenOptions={{ enable: true }}
            allowsPictureInPicture
            startsPictureInPictureAutomatically
          />
        ) : (
          <Blocked item={item} poster={poster} />
        )}
        {ended && next ? <UpNext next={next} onGo={playNext} onReplay={() => seek(0)} /> : null}
      </View>
      <ScrollView ref={scroller} contentContainerClassName="gap-4 px-4 pt-4 pb-12">
        {isOwner ? <OwnerNotice summary={summary} /> : null}
        <View className="gap-1">
          <Text testID="video-title" className="text-xl leading-snug font-black text-ink">
            {summary.title}
          </Text>
          {audience || summary.ageRestricted ? (
            <Text className="text-sm font-bold text-ink-soft">
              {summary.ageRestricted ? "🔞 18+  " : ""}
              {audience ? t(audience) : ""}
            </Text>
          ) : null}
          <Text className="text-xs font-bold text-ink-soft">{videoMeta(summary, false)}</Text>
        </View>

        {channel ? (
          <View className="flex-row items-center gap-3">
            <Pressable
              testID="watch-channel"
              accessibilityRole="link"
              onPress={() => openChannel(channel)}
              className="flex-1 flex-row items-center gap-3"
            >
              <ChannelAvatar channel={channel} size="md" />
              <View className="flex-1">
                <Text numberOfLines={1} className="font-black text-ink">
                  {channel.name}
                </Text>
                <Text numberOfLines={1} className="text-xs text-ink-soft">
                  <SubscriberCount channel={channel} />
                </Text>
              </View>
            </Pressable>
            {isOwner ? (
              <Button
                size="sm"
                variant="secondary"
                onPress={() => openOnWeb(`/studio/videos/${summary.shortId}`)}
              >
                {t("watch.edit")}
              </Button>
            ) : (
              <SubscribeButton
                channel={channel}
                subscribed={item.viewer?.subscribed ?? false}
                notificationLevel={item.viewer?.notificationLevel}
                sourceVideoId={summary.id}
              />
            )}
          </View>
        ) : null}

        <Actions item={item} videoId={videoId} />

        <Pressable
          testID="watch-comments"
          accessibilityRole="button"
          onPress={() => setComments(true)}
          className="flex-row items-center gap-2 rounded-card bg-surface-2 px-4 py-3"
        >
          <MessageCircleIcon />
          <Text className="font-black text-ink">
            <CommentCount count={summary.commentCount} />
          </Text>
        </Pressable>

        <Description item={item} onSeek={seek} />
        <Chapters chapters={item.chapters} time={time} onSeek={seek} />

        {params.list ? (
          <Queue
            title={queue.playlist}
            videos={queue.videos}
            currentId={summary.id}
            list={params.list}
          />
        ) : null}

        <View className="-mx-4 gap-1">
          <Text className="px-4 text-base font-black text-ink">{t("watch.related")}</Text>
          {related.isPending ? (
            <Spinner />
          ) : related.data && related.data.length > 0 ? (
            related.data.map((entry) => (
              <VideoCard key={entry.id} summary={entry} source={TrafficSource.RELATED} />
            ))
          ) : (
            <Text className="px-4 py-4 text-center text-sm text-ink-soft">
              {t("watch.noRelated")}
            </Text>
          )}
        </View>
      </ScrollView>
      <CommentsSheet
        subject={{ type: common.SubjectType.VIDEO, id: summary.id }}
        enabled={item.commentsEnabled}
        visible={comments}
        onClose={() => setComments(false)}
        onSeek={seek}
      />
    </SafeAreaView>
  );
}

function MessageCircleIcon() {
  const { colors } = useTheme();
  return <MessageCircle color={colors.ink} size={20} />;
}

function CommentCount({ count }: { count: bigint }) {
  const { t, i18n } = useTranslation("video");
  return <>{t("app.commentsCount", { formatted: formatCount(i18n.language, count) })}</>;
}

function SubscriberCount({ channel }: { channel: common.ChannelSummary }) {
  const { t, i18n } = useTranslation("video");
  return (
    <>
      {t("channel.subscribers", {
        count: Number(channel.subscriberCount),
        formatted: formatCount(i18n.language, channel.subscriberCount),
      })}
    </>
  );
}

/** What stands in for the player when the video may not be played: age, sign-in, or not ready yet. */
function Blocked({ item, poster }: { item: video.Video; poster: string | undefined }) {
  const { t } = useTranslation("video");
  const kind = blockKind(item);
  return (
    <View testID="video-blocked" className="flex-1 items-center justify-center p-6">
      {poster ? (
        <Image
          source={poster}
          style={{ position: "absolute", inset: 0, opacity: 0.35 }}
          contentFit="cover"
          blurRadius={20}
        />
      ) : null}
      <Text className="text-3xl">{kind === "age" ? "🔞" : kind === "failed" ? "😢" : "⏳"}</Text>
      <Text className="mt-2 text-center text-base font-black text-white">
        {t(`watch.blocked.${kind}Title`)}
      </Text>
      <Text className="mt-1 text-center text-sm text-white/80">
        {t(`watch.blocked.${kind}Body`)}
      </Text>
    </View>
  );
}

/** What only the owner is told above the title: a draft, scheduled, or private video. */
function OwnerNotice({ summary }: { summary: video.VideoSummary }) {
  const { t, i18n } = useTranslation("video");
  const text =
    summary.status === VideoStatus.DRAFT
      ? t("watch.owner.draft")
      : summary.status === VideoStatus.SCHEDULED && summary.scheduledAt
        ? t("watch.owner.scheduled", {
            date: new Date(toMillis(summary.scheduledAt)).toLocaleString(i18n.language, {
              month: "long",
              day: "numeric",
              hour: "2-digit",
              minute: "2-digit",
            }),
          })
        : summary.visibility === common.Visibility.PRIVATE
          ? t("watch.owner.private")
          : "";
  if (!text) return null;
  return (
    <Pressable
      testID="owner-notice"
      accessibilityRole="link"
      onPress={() => openOnWeb(`/studio/videos/${summary.shortId}`)}
      className="gap-1 rounded-2xl bg-sun px-4 py-3"
    >
      <Text className="text-sm font-bold text-[#17122b]">{text}</Text>
      <Text className="text-sm font-black text-[#17122b] underline">
        {t("watch.owner.openStudio")}
      </Text>
    </Pressable>
  );
}

/** Like / dislike, watch later, save, share — and for the owner, the studio on the web. */
function Actions({ item, videoId }: { item: video.Video; videoId: string }) {
  const { t, i18n } = useTranslation("video");
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const rate = useRate(videoId);
  const saveTo = useSaveTo();
  const summary = item.summary as video.VideoSummary;
  const rating = item.viewer?.rating ?? Rating.NONE;
  const liked = rating === Rating.LIKE;
  const disliked = rating === Rating.DISLIKE;
  const published = summary.status === VideoStatus.PUBLISHED;
  const inWatchLater = item.viewer?.inWatchLater ?? false;

  const toggleWatchLater = async () => {
    const key = videoKeys.video(videoId);
    const flip = (value: boolean) =>
      queryClient.setQueryData<video.Video | null>(key, (current) =>
        current?.viewer
          ? { ...current, viewer: { ...current.viewer, inWatchLater: value } }
          : current,
      );
    flip(!inWatchLater);
    if (!(await saveTo("WL", summary, !inWatchLater))) flip(inWatchLater);
  };
  const share = () =>
    void Share.share({ message: `${summary.title}\n${webLink(`/watch/${summary.shortId}`)}` });

  return (
    <ScrollView horizontal showsHorizontalScrollIndicator={false} className="-mx-4">
      <View className="flex-row gap-2 px-4">
        <Chip
          testID="video-like"
          selected={liked}
          disabled={!published}
          label={t("watch.like")}
          onPress={() => void rate(item, liked ? Rating.NONE : Rating.LIKE)}
          icon={<ThumbsUp color={colors.ink} fill={liked ? colors.ink : "transparent"} size={18} />}
        >
          <Text testID="video-like-count" className="font-black text-ink">
            {formatCount(i18n.language, summary.likeCount)}
          </Text>
        </Chip>
        <Chip
          testID="video-dislike"
          selected={disliked}
          disabled={!published}
          label={t("watch.dislike")}
          onPress={() => void rate(item, disliked ? Rating.NONE : Rating.DISLIKE)}
          icon={
            <ThumbsDown color={colors.ink} fill={disliked ? colors.ink : "transparent"} size={18} />
          }
        />
        <Chip
          testID="video-watch-later"
          selected={inWatchLater}
          label={inWatchLater ? t("save.inWatchLater") : t("save.watchLater")}
          onPress={() => void toggleWatchLater()}
          icon={<Clock3 color={inWatchLater ? colors.brand : colors.ink} size={18} />}
          text
        />
        <Chip
          testID="video-save"
          label={t("save.action")}
          onPress={() => openSaveSheet(summary)}
          icon={<ListPlus color={colors.ink} size={18} />}
          text
        />
        <Chip
          testID="video-share"
          label={t("app.share")}
          onPress={share}
          icon={<Share2 color={colors.ink} size={18} />}
          text
        />
        {item.viewer?.isOwner ? (
          <Chip
            testID="video-studio"
            label={t("watch.analytics")}
            onPress={() => openOnWeb(`/studio/videos/${summary.shortId}?tab=analytics`)}
            icon={<Pencil color={colors.ink} size={18} />}
            text
          />
        ) : null}
      </View>
    </ScrollView>
  );
}

function Chip({
  testID,
  label,
  icon,
  selected,
  disabled,
  text,
  onPress,
  children,
}: {
  testID: string;
  label: string;
  icon: ReactNode;
  selected?: boolean;
  disabled?: boolean;
  /** Show the label next to the icon. */
  text?: boolean;
  onPress(): void;
  children?: ReactNode;
}) {
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ selected: Boolean(selected), disabled: Boolean(disabled) }}
      disabled={disabled}
      onPress={onPress}
      className={`h-10 flex-row items-center gap-1.5 rounded-full px-4 ${selected ? "bg-lime" : "bg-surface-2"} ${disabled ? "opacity-50" : ""}`}
    >
      {icon}
      {text ? <Text className="text-sm font-black text-ink">{label}</Text> : null}
      {children}
    </Pressable>
  );
}

/** The playlist being played, the current video marked; a tap jumps anywhere in it. */
function Queue({
  title,
  videos,
  currentId,
  list,
}: {
  title: video.Playlist | undefined;
  videos: video.VideoSummary[];
  currentId: string;
  list: string;
}) {
  const { t } = useTranslation("video");
  const index = videos.findIndex((entry) => entry.id === currentId);
  return (
    <View testID="watch-queue" className="gap-2 rounded-card bg-surface-2 py-3">
      <Text className="px-4 font-black text-ink">
        {title ? playlistTitle(title, t) : t("playlist.queue")}
        {index >= 0 ? `  ${index + 1} / ${videos.length}` : ""}
      </Text>
      <ScrollView horizontal showsHorizontalScrollIndicator={false}>
        <View className="flex-row gap-2 px-4">
          {videos.map((entry) => (
            <Pressable
              key={entry.id}
              accessibilityRole="button"
              accessibilityLabel={entry.title}
              onPress={() =>
                router.replace({
                  pathname: "/watch/[videoId]",
                  params: { videoId: entry.shortId || entry.id, list },
                })
              }
              className={`w-36 gap-1 rounded-xl p-1 ${entry.id === currentId ? "bg-brand-soft" : ""}`}
            >
              <View className="aspect-video overflow-hidden rounded-lg bg-surface-3">
                <QueueThumb summary={entry} />
              </View>
              <Text numberOfLines={2} className="text-xs font-bold text-ink">
                {entry.title}
              </Text>
            </Pressable>
          ))}
        </View>
      </ScrollView>
    </View>
  );
}

function QueueThumb({ summary }: { summary: video.VideoSummary }) {
  const uri = mediaUrl(imageUrl(summary.thumbnail, 144));
  return uri ? <Image source={uri} style={{ flex: 1 }} contentFit="cover" /> : null;
}

/** Over the picture once a video has ended: the next one, starting by itself unless cancelled. */
function UpNext({
  next,
  onGo,
  onReplay,
}: {
  next: video.VideoSummary;
  onGo(): void;
  onReplay(): void;
}) {
  const { t } = useTranslation("video");
  const [remaining, setRemaining] = useState<number | null>(UP_NEXT_SECONDS);
  const go = useRef(onGo);
  useEffect(() => {
    go.current = onGo;
  });
  useEffect(() => {
    if (remaining === null) return;
    if (remaining <= 0) {
      go.current();
      return;
    }
    const timer = setTimeout(
      () => setRemaining((value) => (value === null ? null : value - 1)),
      1000,
    );
    return () => clearTimeout(timer);
  }, [remaining]);
  return (
    <View
      testID="up-next"
      className="absolute inset-0 items-center justify-center gap-2 bg-black/80 px-6"
    >
      <Text className="text-xs font-bold text-white/80">
        {remaining === null ? t("watch.upNext") : t("watch.upNextIn", { seconds: remaining })}
      </Text>
      <Text numberOfLines={2} className="text-center text-base font-black text-white">
        {next.title}
      </Text>
      <View className="flex-row gap-2">
        {remaining !== null ? (
          <UpNextButton label={t("actions.cancel")} onPress={() => setRemaining(null)} />
        ) : null}
        <UpNextButton label={t("player.replay")} onPress={onReplay} />
        <UpNextButton testID="up-next-play" label={t("watch.playNext")} onPress={onGo} strong />
      </View>
    </View>
  );
}

function UpNextButton({
  label,
  onPress,
  strong,
  testID,
}: {
  label: string;
  onPress(): void;
  strong?: boolean;
  testID?: string;
}) {
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      onPress={onPress}
      className={`h-9 justify-center rounded-full px-4 ${strong ? "bg-white" : "bg-white/20"}`}
    >
      <Text className={`text-sm font-black ${strong ? "text-[#17122b]" : "text-white"}`}>
        {label}
      </Text>
    </Pressable>
  );
}
