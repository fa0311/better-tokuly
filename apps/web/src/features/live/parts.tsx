import { Link } from "@tanstack/react-router";
import { type common, live, toMillis } from "@tokuly/api";
import { countdownParts } from "@tokuly/core/live";
import { BadgeCheck, CalendarClock, Eye, Radio } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { accountTimeZone } from "#/lib/account-time";
import { cn } from "#/lib/cn";
import { categoryOf } from "./labels";
import { clock } from "./player/live-player";

const { StreamStatus } = live;

/** The current time, ticking: for countdowns, uptimes and highlights that run out. */
export function useNow(intervalMs = 1000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(timer);
  }, [intervalMs]);
  return now;
}

/** A count for chips: exact while small, compact (`1.2万` / `12K`) after. */
export function formatCount(value: number | bigint, language: string): string {
  const count = Number(value);
  if (count < 10_000) return count.toLocaleString(language);
  return new Intl.NumberFormat(language, { notation: "compact", maximumFractionDigits: 1 }).format(
    count,
  );
}

/** A moment on the account's clock, as people read it: `11/23(月) 21:00`. */
export function formatMoment(
  ms: number,
  language: string,
  timeZone: string = accountTimeZone(),
): string {
  return new Date(ms).toLocaleString(language, {
    month: "numeric",
    day: "numeric",
    weekday: "short",
    hour: "2-digit",
    minute: "2-digit",
    timeZone,
  });
}

/** The smallest stored rendition that still covers `pixels` on a 2x screen. */
export function imageUrl(image: common.Image | undefined, pixels: number): string | undefined {
  if (!image) return undefined;
  const fitting = image.variants
    .filter((variant) => variant.size >= pixels * 2)
    .sort((a, b) => a.size - b.size)[0];
  return fitting?.url ?? (image.url || undefined);
}

export function ChannelFace({
  channel,
  size = "sm",
  className,
}: {
  channel: common.ChannelSummary;
  size?: "xs" | "sm" | "md" | "lg";
  className?: string;
}) {
  return (
    <Avatar
      user={{
        id: channel.id,
        displayName: channel.name,
        handle: channel.handle,
        avatar: channel.icon,
        deleted: false,
      }}
      size={size}
      className={className}
    />
  );
}

export function ChannelTitle({
  channel,
  className,
}: {
  channel: common.ChannelSummary;
  className?: string;
}) {
  const { t } = useTranslation("live");
  return (
    <span className={cn("inline-flex min-w-0 items-center gap-1", className)}>
      <span className="truncate">
        <EmojiText text={channel.name} />
      </span>
      {channel.verified && (
        <BadgeCheck className="size-4 shrink-0 text-brand" role="img" aria-label={t("verified")} />
      )}
    </span>
  );
}

/** The red "LIVE" mark. The word says it; the colour only backs it up. */
export function LiveMark({ className }: { className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex h-6 shrink-0 items-center gap-1 rounded-md bg-live px-1.5 text-[11px] font-black tracking-wide text-white",
        className,
      )}
    >
      <Radio className="size-3.5" />
      LIVE
    </span>
  );
}

/** How many are watching, with an eye. */
export function ViewerCount({ count, className }: { count: bigint | number; className?: string }) {
  const { t, i18n } = useTranslation("live");
  return (
    <span
      className={cn("inline-flex items-center gap-1 text-sm font-bold", className)}
      data-testid="viewer-count"
      data-count={String(count)}
    >
      <Eye className="size-4" aria-hidden />
      {t("viewers", { formatted: formatCount(count, i18n.language) })}
    </span>
  );
}

/** What is left until a scheduled start: `あと 2日 3時間`, `あと 4:59`, then "まもなく". */
export function Countdown({ targetMs, className }: { targetMs: number; className?: string }) {
  const { t } = useTranslation("live");
  const now = useNow();
  const left = countdownParts(targetMs, now);
  const text =
    left.total === 0
      ? t("countdown.soon")
      : left.days > 0
        ? t("countdown.days", { days: left.days, hours: left.hours })
        : left.hours > 0
          ? t("countdown.hours", { hours: left.hours, minutes: left.minutes })
          : t("countdown.clock", { clock: clock(left.total) });
  return (
    <span className={cn("tabular-nums", className)} data-testid="countdown">
      {text}
    </span>
  );
}

/** A stream's picture: its thumbnail, or a tinted card with the channel's face. */
export function StreamPicture({
  stream,
  className,
  children,
}: {
  stream: live.StreamSummary;
  className?: string;
  children?: ReactNode;
}) {
  const url = imageUrl(stream.thumbnail, 360);
  return (
    <span
      className={cn(
        "relative block aspect-video overflow-hidden rounded-2xl bg-surface-3",
        className,
      )}
      style={
        stream.thumbnail?.dominantColor ? { background: stream.thumbnail.dominantColor } : undefined
      }
    >
      {url ? (
        <img src={url} alt="" loading="lazy" className="size-full object-cover" />
      ) : (
        <span className="grid size-full place-items-center bg-gradient-to-br from-brand-soft to-surface-3">
          {stream.channel && <ChannelFace channel={stream.channel} size="lg" />}
        </span>
      )}
      {children}
    </span>
  );
}

/**
 * A stream as a card that leads to its page: on air with its audience,
 * scheduled with a countdown, or over.
 */
export function StreamCard({
  stream,
  layout = "grid",
  className,
}: {
  stream: live.StreamSummary;
  layout?: "grid" | "row";
  className?: string;
}) {
  const { t, i18n } = useTranslation("live");
  const category = categoryOf(stream.category);
  const onAir = stream.status === StreamStatus.LIVE;
  const scheduledMs = stream.scheduledAt ? toMillis(stream.scheduledAt) : undefined;
  return (
    <Link
      to="/live/$streamId"
      params={{ streamId: stream.shortId }}
      className={cn(
        "group flex min-w-0 gap-3 rounded-2xl outline-none focus-visible:ring-2 focus-visible:ring-brand",
        layout === "grid" ? "flex-col" : "items-start",
        className,
      )}
      data-testid="stream-card"
    >
      <StreamPicture stream={stream} className={layout === "row" ? "w-40 shrink-0" : undefined}>
        {onAir && <LiveMark className="absolute top-2 left-2" />}
        {onAir && (
          <span className="absolute right-2 bottom-2 rounded-md bg-black/70 px-1.5 py-0.5 text-white">
            <ViewerCount count={stream.viewerCount} className="text-xs" />
          </span>
        )}
        {stream.status === StreamStatus.UPCOMING && scheduledMs !== undefined && (
          <span className="absolute right-2 bottom-2 flex items-center gap-1 rounded-md bg-black/70 px-1.5 py-0.5 text-xs font-black text-white">
            <CalendarClock className="size-3.5" />
            <Countdown targetMs={scheduledMs} />
          </span>
        )}
        {stream.status === StreamStatus.ENDED && (
          <span className="absolute top-2 left-2 rounded-md bg-black/70 px-1.5 py-0.5 text-[11px] font-black text-white">
            {t("status.ended")}
          </span>
        )}
      </StreamPicture>
      <span className="flex min-w-0 flex-1 gap-2.5">
        {layout === "grid" && stream.channel && <ChannelFace channel={stream.channel} />}
        <span className="min-w-0 flex-1">
          <span className="line-clamp-2 font-black leading-snug group-hover:text-brand-strong">
            <EmojiText text={stream.title} />
          </span>
          {stream.channel && (
            <ChannelTitle channel={stream.channel} className="mt-0.5 text-sm text-ink-soft" />
          )}
          <span className="mt-0.5 flex flex-wrap items-center gap-x-2 text-xs font-bold text-ink-faint">
            {stream.game ? (
              <span className="truncate">🎮 {stream.game}</span>
            ) : (
              category && (
                <span>
                  {category.emoji} {t(`category.${category.key}`)}
                </span>
              )
            )}
            {stream.status === StreamStatus.UPCOMING && scheduledMs !== undefined && (
              <span>{formatMoment(scheduledMs, i18n.language)}</span>
            )}
            {stream.status === StreamStatus.ENDED && stream.endedAt && (
              <span>{formatMoment(toMillis(stream.endedAt), i18n.language)}</span>
            )}
          </span>
        </span>
      </span>
    </Link>
  );
}

export function StreamCardSkeleton() {
  return (
    <div className="flex flex-col gap-3">
      <Skeleton className="aspect-video w-full" />
      <div className="flex gap-2.5">
        <Skeleton className="size-8 shrink-0" />
        <div className="flex-1 space-y-2">
          <Skeleton className="h-4 w-4/5 rounded-full" />
          <Skeleton className="h-3 w-2/5 rounded-full" />
        </div>
      </div>
    </div>
  );
}

/** The responsive grid stream cards sit in. */
export function StreamGrid({ children }: { children: ReactNode }) {
  return (
    <div className="grid grid-cols-1 gap-x-4 gap-y-6 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
      {children}
    </div>
  );
}
