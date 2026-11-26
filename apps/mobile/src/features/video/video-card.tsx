import { type common, toMillis, video } from "@tokuly/api";
import { formatAge, formatClock, formatCount, imageUrl } from "@tokuly/core/video";
import { Image } from "expo-image";
import { router } from "expo-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { ActionSheet, type SheetAction } from "#/components/action-sheet";
import { Avatar, type AvatarSize } from "#/components/avatar";
import { mediaUrl } from "#/lib/config";
import i18n from "#/lib/i18n";
import { useSaveTo } from "./data";
import { openSaveSheet } from "./save-sheet";

const { TrafficSource, VideoOrigin } = video;

/** Opens a video's watch screen, saying where the viewer came from (and the playlist it plays in). */
export function openVideo(
  summary: Pick<video.VideoSummary, "id" | "shortId">,
  source: video.TrafficSource,
  list?: string,
): void {
  router.push({
    pathname: "/watch/[videoId]",
    params: {
      videoId: summary.shortId || summary.id,
      via: String(source),
      ...(list ? { list } : {}),
    },
  });
}

/** The traffic source a watch link carries in `via`; anything else is a direct visit. */
export function sourceOf(via: string | undefined): video.TrafficSource {
  const value = Number(via);
  return Number.isInteger(value) && value in TrafficSource && value !== 0
    ? (value as video.TrafficSource)
    : TrafficSource.DIRECT;
}

export function openChannel(channel: Pick<common.ChannelSummary, "handle">): void {
  router.push({ pathname: "/c/[handle]", params: { handle: channel.handle } });
}

/** A channel drawn as a face: channels have no user, the avatar takes their icon and name. */
export function ChannelAvatar({
  channel,
  size = "sm",
}: {
  channel: common.ChannelSummary | undefined;
  size?: AvatarSize;
}) {
  return (
    <Avatar
      size={size}
      user={
        channel && {
          id: channel.id,
          displayName: channel.name,
          handle: channel.handle,
          avatar: channel.icon,
          deleted: false,
        }
      }
    />
  );
}

/** "チャンネル · 1.2万回視聴 · 3日前". */
export function videoMeta(summary: video.VideoSummary, withChannel = true): string {
  const parts: string[] = [];
  if (withChannel && summary.channel) parts.push(summary.channel.name);
  parts.push(
    i18n.t("video:app.views", { formatted: formatCount(i18n.language, summary.viewCount) }),
  );
  if (summary.publishedAt) parts.push(formatAge(i18n, toMillis(summary.publishedAt)));
  return parts.join(" · ");
}

/** The picture: thumbnail, length, how far the viewer got, and a live archive's mark. */
export function Thumbnail({ summary, width }: { summary: video.VideoSummary; width: number }) {
  const { t } = useTranslation("video");
  const uri = mediaUrl(imageUrl(summary.thumbnail, width));
  const progress =
    summary.durationSeconds > 0
      ? summary.watched
        ? 1
        : Math.min(1, summary.resumePositionSeconds / summary.durationSeconds)
      : 0;
  return (
    <View className="aspect-video overflow-hidden rounded-2xl bg-surface-3" style={{ width }}>
      {uri ? <Image source={uri} style={{ flex: 1 }} contentFit="cover" /> : null}
      {summary.origin === VideoOrigin.LIVE_ARCHIVE && (
        <Text className="absolute top-1.5 left-1.5 rounded-md bg-live px-1.5 text-[10px] leading-5 font-black text-white">
          {t("card.liveArchive")}
        </Text>
      )}
      {summary.durationSeconds > 0 && (
        <Text className="absolute right-1.5 bottom-1.5 rounded-md bg-black/75 px-1.5 text-xs leading-5 font-black text-white">
          {formatClock(summary.durationSeconds)}
        </Text>
      )}
      {progress > 0 && (
        <View className="absolute inset-x-0 bottom-0 h-1 bg-white/40">
          <View
            testID="video-progress"
            className="h-1 bg-live"
            style={{ width: `${progress * 100}%` }}
          />
        </View>
      )}
    </View>
  );
}

/**
 * A video in a list: tap to watch, hold for the menu (watch later, save, and
 * what the list adds: "not interested", remove from history…). `layout`
 * "large" is the feeds' full-width card, "row" a compact line.
 */
export function VideoCard({
  summary,
  source,
  list,
  layout = "row",
  width,
  menu = [],
}: {
  summary: video.VideoSummary;
  source: video.TrafficSource;
  /** The playlist the card plays in. */
  list?: string;
  layout?: "large" | "row";
  /** The screen's width, for the large card's picture. */
  width?: number;
  menu?: SheetAction[];
}) {
  const { t } = useTranslation("video");
  const [open, setOpen] = useState(false);
  const saveTo = useSaveTo();
  const actions: SheetAction[] = [
    {
      key: "watch-later",
      label: t("save.watchLater"),
      onPress: () => void saveTo("WL", summary, true),
    },
    { key: "save", label: t("save.toPlaylist"), onPress: () => openSaveSheet(summary) },
    ...menu,
  ];
  const testID = `video-card-${summary.shortId || summary.id}`;
  const onPress = () => openVideo(summary, source, list);
  const sheet = (
    <ActionSheet
      visible={open}
      title={summary.title}
      actions={actions}
      onClose={() => setOpen(false)}
    />
  );

  if (layout === "large") {
    return (
      <Pressable
        testID={testID}
        accessibilityRole="button"
        accessibilityLabel={summary.title}
        onPress={onPress}
        onLongPress={() => setOpen(true)}
        className="gap-2.5 px-4 pb-5"
      >
        <Thumbnail summary={summary} width={(width ?? 360) - 32} />
        <View className="flex-row gap-3">
          <Pressable
            accessibilityRole="link"
            accessibilityLabel={summary.channel?.name}
            onPress={() => summary.channel && openChannel(summary.channel)}
          >
            <ChannelAvatar channel={summary.channel} size="md" />
          </Pressable>
          <View className="flex-1 gap-0.5">
            <Text numberOfLines={2} className="text-base leading-snug font-black text-ink">
              {summary.title}
            </Text>
            <Text numberOfLines={1} className="text-xs font-bold text-ink-soft">
              {videoMeta(summary)}
            </Text>
          </View>
        </View>
        {sheet}
      </Pressable>
    );
  }
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      accessibilityLabel={summary.title}
      onPress={onPress}
      onLongPress={() => setOpen(true)}
      className="flex-row gap-3 px-4 py-2"
    >
      <Thumbnail summary={summary} width={160} />
      <View className="flex-1 gap-1">
        <Text numberOfLines={2} className="text-sm leading-snug font-black text-ink">
          {summary.title}
        </Text>
        <Text numberOfLines={2} className="text-xs font-bold text-ink-soft">
          {videoMeta(summary)}
        </Text>
      </View>
      {sheet}
    </Pressable>
  );
}
