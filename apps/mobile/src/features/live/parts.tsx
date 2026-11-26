import { type common, live, toMillis } from "@tokuly/api";
import { compactCount } from "@tokuly/core/engagement";
import { countdownParts } from "@tokuly/core/live";
import { imageUrl } from "@tokuly/core/shorts";
import { Image } from "expo-image";
import { router } from "expo-router";
import { Eye, Radio } from "lucide-react-native";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Avatar, type AvatarSize } from "#/components/avatar";
import { mediaUrl } from "#/lib/config";

const { StreamStatus } = live;

/** The clock, ticking every `intervalMs`. */
export function useNow(intervalMs = 1000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(timer);
  }, [intervalMs]);
  return now;
}

/** `m:ss`, or `h:mm:ss` past an hour. */
export function clock(totalSeconds: number): string {
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = String(totalSeconds % 60).padStart(2, "0");
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${seconds}`
    : `${minutes}:${seconds}`;
}

/** A moment as people read it, on the phone's clock: `11/23(月) 21:00`. */
export function formatMoment(ms: number, language: string): string {
  return new Date(ms).toLocaleString(language, {
    month: "numeric",
    day: "numeric",
    weekday: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** A stream's picture, or `undefined` when the streamer chose none. */
export function thumbnailOf(summary: live.StreamSummary, pixels = 320): string | undefined {
  return mediaUrl(imageUrl(summary.thumbnail, pixels));
}

/** "あと 3:20", counting down to `targetMs`. */
export function Countdown({ targetMs, className }: { targetMs: number; className?: string }) {
  const { t } = useTranslation("live");
  const left = countdownParts(targetMs, useNow());
  const text =
    left.total === 0
      ? t("countdown.soon")
      : left.days > 0
        ? t("countdown.days", { days: left.days, hours: left.hours })
        : left.hours > 0
          ? t("countdown.hours", { hours: left.hours, minutes: left.minutes })
          : t("countdown.clock", { clock: clock(left.total) });
  return (
    <Text testID="countdown" className={className} style={{ fontVariant: ["tabular-nums"] }}>
      {text}
    </Text>
  );
}

/** The red mark of anything on air right now. */
export function LiveMark() {
  const { t } = useTranslation("live");
  return (
    <View className="flex-row items-center gap-1 rounded-md bg-live px-1.5 py-0.5">
      <Radio size={11} color="#ffffff" />
      <Text className="text-[11px] font-black text-white">{t("player.live")}</Text>
    </View>
  );
}

/** "12人が視聴中", with an eye. */
export function ViewerCount({ count, light }: { count: bigint; light?: boolean }) {
  const { t, i18n } = useTranslation("live");
  return (
    <View className="flex-row items-center gap-1">
      <Eye size={13} color={light ? "#ffffff" : "#5d5778"} />
      <Text className={`text-xs font-bold ${light ? "text-white" : "text-ink-soft"}`}>
        {t("viewers", { formatted: compactCount(Number(count), i18n.language) })}
      </Text>
    </View>
  );
}

/** A channel's icon in the shape of a person's avatar. */
export function ChannelFace({
  channel,
  size = "sm",
}: {
  channel: common.ChannelSummary;
  size?: AvatarSize;
}) {
  return (
    <Avatar
      size={size}
      user={{
        id: channel.id,
        displayName: channel.name,
        handle: channel.handle,
        avatar: channel.icon,
        deleted: false,
      }}
    />
  );
}

/** What a card says under its title: on air with its audience, when it starts, or that it is over. */
function CardStatus({ summary }: { summary: live.StreamSummary }) {
  const { t } = useTranslation("live");
  if (summary.status === StreamStatus.LIVE) return <ViewerCount count={summary.viewerCount} />;
  if (summary.status === StreamStatus.ENDED)
    return <Text className="text-xs font-bold text-ink-soft">{t("status.ended")}</Text>;
  return summary.scheduledAt ? (
    <Countdown
      targetMs={toMillis(summary.scheduledAt)}
      className="text-xs font-bold text-brand-strong"
    />
  ) : (
    <Text className="text-xs font-bold text-ink-soft">{t("status.waiting")}</Text>
  );
}

/**
 * A stream as a card: its picture with the LIVE mark, the title, the
 * channel, and whether it is on air, coming or over. A tap opens it.
 */
export function StreamCard({
  summary,
  width,
  testID,
}: {
  summary: live.StreamSummary;
  /** Fixed width for a shelf; full width otherwise. */
  width?: number;
  testID?: string;
}) {
  const thumbnail = thumbnailOf(summary);
  const channel = summary.channel;
  return (
    <Pressable
      testID={testID ?? `stream-card-${summary.shortId}`}
      accessibilityRole="button"
      accessibilityLabel={summary.title}
      onPress={() => router.push(`/live/${summary.shortId}`)}
      style={width ? { width } : undefined}
      className="gap-2"
    >
      <View className="aspect-video overflow-hidden rounded-2xl bg-[#17122b]">
        {thumbnail ? (
          <Image source={thumbnail} style={{ flex: 1 }} contentFit="cover" />
        ) : (
          <View className="flex-1 items-center justify-center">
            <Radio size={32} color="#ffffff80" />
          </View>
        )}
        {summary.status === StreamStatus.LIVE && (
          <View className="absolute top-2 left-2">
            <LiveMark />
          </View>
        )}
      </View>
      <View className="flex-row gap-2.5">
        {channel && <ChannelFace channel={channel} size="sm" />}
        <View className="min-w-0 flex-1 gap-0.5">
          <Text numberOfLines={2} className="text-sm leading-snug font-black text-ink">
            {summary.title}
          </Text>
          {channel && (
            <Text numberOfLines={1} className="text-xs font-bold text-ink-soft">
              {channel.name}
            </Text>
          )}
          <CardStatus summary={summary} />
        </View>
      </View>
    </Pressable>
  );
}
