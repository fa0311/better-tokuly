import { toMillis, type video } from "@tokuly/api";
import {
  categoryKey,
  chapterAt,
  descriptionParts,
  formatClock,
  formatCount,
} from "@tokuly/core/video";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Linking, Pressable, Text, View } from "react-native";

/** A creator's free text with its web addresses (the browser) and clock times (the player) tappable. */
export function LinkedText({
  text,
  onSeek,
  className,
  numberOfLines,
}: {
  text: string;
  onSeek?(seconds: number): void;
  className?: string;
  numberOfLines?: number;
}) {
  return (
    <Text className={className} numberOfLines={numberOfLines}>
      {descriptionParts(text).map((part, index) => {
        const key = `${index}-${part.text}`;
        if (part.kind === "link")
          return (
            <Text
              key={key}
              accessibilityRole="link"
              className="font-bold text-brand"
              onPress={() => void Linking.openURL(part.text)}
            >
              {part.text}
            </Text>
          );
        if (part.kind === "time" && onSeek)
          return (
            <Text
              key={key}
              accessibilityRole="button"
              className="font-bold text-brand"
              onPress={() => onSeek(part.seconds)}
            >
              {part.text}
            </Text>
          );
        return <Text key={key}>{part.text}</Text>;
      })}
    </Text>
  );
}

/**
 * The box under the title: views and the day it went up, what the creator
 * wrote (folded until asked for), genre, game and tags.
 */
export function Description({
  item,
  onSeek,
}: {
  item: video.Video;
  onSeek(seconds: number): void;
}) {
  const { t, i18n } = useTranslation("video");
  const [open, setOpen] = useState(false);
  const summary = item.summary;
  const published = summary?.publishedAt ? new Date(toMillis(summary.publishedAt)) : undefined;
  const category = summary ? categoryKey(summary.category) : undefined;
  const hasMore =
    item.description.length > 100 ||
    item.description.split("\n").length > 2 ||
    item.tags.length > 0 ||
    Boolean(category || item.game);
  return (
    <Pressable
      testID="video-description"
      accessibilityRole="button"
      accessibilityState={{ expanded: open }}
      accessibilityLabel={t("watch.description")}
      disabled={!hasMore}
      onPress={() => setOpen((value) => !value)}
      className="gap-2 rounded-card bg-surface-2 p-4"
    >
      <Text className="text-sm font-black text-ink">
        {t("app.views", { formatted: formatCount(i18n.language, summary?.viewCount ?? 0n) })}
        {published
          ? `  ${published.toLocaleDateString(i18n.language, {
              year: "numeric",
              month: "long",
              day: "numeric",
            })}`
          : ""}
      </Text>
      {item.description ? (
        <LinkedText
          text={item.description}
          onSeek={onSeek}
          numberOfLines={open ? undefined : 2}
          className="text-[15px] leading-relaxed text-ink"
        />
      ) : (
        <Text className="text-ink-faint">{t("watch.noDescription")}</Text>
      )}
      {open && (category || item.game) ? (
        <View className="flex-row flex-wrap gap-2">
          {category ? <Chip text={t(category)} /> : null}
          {item.game ? <Chip text={`🎮 ${item.game}`} /> : null}
        </View>
      ) : null}
      {open && item.tags.length > 0 ? (
        <Text className="text-sm font-bold text-brand">
          {item.tags.map((tag) => `#${tag}`).join("  ")}
        </Text>
      ) : null}
      {hasMore ? (
        <Text className="text-sm font-black text-ink-soft">
          {open ? t("watch.showLess") : t("watch.showMore")}
        </Text>
      ) : null}
    </Pressable>
  );
}

function Chip({ text }: { text: string }) {
  return (
    <Text className="rounded-full bg-surface px-3 py-1 text-xs font-black text-ink-soft">
      {text}
    </Text>
  );
}

/** The chapters as a list that jumps to each; the one playing is marked. */
export function Chapters({
  chapters,
  time,
  onSeek,
}: {
  chapters: video.Chapter[];
  time: number;
  onSeek(seconds: number): void;
}) {
  const { t } = useTranslation("video");
  if (chapters.length === 0) return null;
  const playing = chapterAt(chapters, time);
  return (
    <View testID="video-chapters" className="gap-1">
      <Text className="px-1 text-base font-black text-ink">{t("watch.chapters")}</Text>
      {chapters.map((chapter) => {
        const current = chapter === playing;
        return (
          <Pressable
            key={chapter.startSeconds}
            testID={`chapter-${Math.round(chapter.startSeconds)}`}
            accessibilityRole="button"
            accessibilityState={{ selected: current }}
            onPress={() => onSeek(chapter.startSeconds)}
            className={`flex-row items-center gap-3 rounded-xl px-2 py-2 ${current ? "bg-surface-2" : ""}`}
          >
            <Text className="w-16 rounded-md bg-brand-soft px-1.5 py-0.5 text-center text-xs font-black text-brand-strong">
              {formatClock(chapter.startSeconds)}
            </Text>
            <Text numberOfLines={1} className="flex-1 font-bold text-ink">
              {chapter.title}
            </Text>
            {current ? (
              <Text className="text-xs font-black text-brand">{t("app.playing")}</Text>
            ) : null}
          </Pressable>
        );
      })}
    </View>
  );
}
