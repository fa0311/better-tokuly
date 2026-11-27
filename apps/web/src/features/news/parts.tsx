import { Link } from "@tanstack/react-router";
import { feed, type Timestamp, toMillis } from "@tokuly/api";
import { Dices, Globe, ThumbsDown, ThumbsUp } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "#/lib/cn";
import { compactAge } from "#/lib/format";
import { linkPreviewImage } from "#/lib/link-preview-image";
import { useFeedback } from "./api";
import { toggledFeedback } from "./requests";

const WEEK_MS = 7 * 24 * 60 * 60 * 1000;

/** "3時間前" for the last week, a date before that. */
export function PublishedAt({ at, className }: { at: Timestamp | undefined; className?: string }) {
  const { i18n } = useTranslation("news");
  const millis = toMillis(at);
  if (!millis) return null;
  const date = new Date(millis);
  const recent = Date.now() - millis < WEEK_MS;
  const sameYear = date.getFullYear() === new Date().getFullYear();
  const label = recent
    ? compactAge(millis)
    : new Intl.DateTimeFormat(
        i18n.language,
        sameYear
          ? { month: "short", day: "numeric" }
          : { year: "numeric", month: "short", day: "numeric" },
      ).format(date);
  return (
    <time dateTime={date.toISOString()} className={className}>
      {label}
    </time>
  );
}

/**
 * A source's icon: its favicon, or a globe when it has none (or it fails to load).
 * Like every picture of another site, it is only loaded from this instance
 * (`linkPreviewImage`): asking the site itself would tell it who reads.
 */
export function SourceIcon({
  source,
  className,
}: {
  source: Pick<feed.NewsSource, "faviconUrl"> | undefined;
  className?: string;
}) {
  const [broken, setBroken] = useState(false);
  const src = linkPreviewImage(source?.faviconUrl);
  if (!src || broken)
    return <Globe className={cn("size-4 shrink-0 text-ink-faint", className)} aria-hidden />;
  return (
    <img
      src={src}
      alt=""
      className={cn("size-4 shrink-0 rounded-[4px]", className)}
      loading="lazy"
      decoding="async"
      onError={() => setBroken(true)}
      // Some sites answer the icon's address with an empty picture.
      onLoad={(event) => event.currentTarget.naturalWidth === 0 && setBroken(true)}
      draggable={false}
    />
  );
}

/** The marker of an article that was picked at random rather than for the reader's interests. */
export function RandomPickChip({ className }: { className?: string }) {
  const { t } = useTranslation("news");
  return (
    <span
      title={t("random.explain")}
      className={cn(
        "inline-flex h-6 shrink-0 items-center gap-1 rounded-full border-2 border-ink bg-sun px-2 text-[11px] font-black whitespace-nowrap text-[#17122b]",
        className,
      )}
    >
      <Dices className="size-3.5" />
      {t("random.label")}
    </span>
  );
}

/** A tag as a link to the list filtered by it. */
export function TagLink({ tag, className }: { tag: string; className?: string }) {
  return (
    <Link
      to="/news"
      search={{ tag }}
      className={cn(
        "inline-flex h-7 max-w-full items-center rounded-full bg-surface-2 px-2.5 text-xs font-bold text-ink-soft hover:bg-surface-3 hover:text-ink",
        className,
      )}
    >
      <span className="truncate">#{tag}</span>
    </Link>
  );
}

/** "More like this" / "less like this": two toggles; pressing the chosen one again takes the feedback back. */
export function FeedbackButtons({
  article,
  compact,
  className,
}: {
  article: feed.NewsArticle /** Icons only (cards). */;
  compact?: boolean;
  className?: string;
}) {
  const { t } = useTranslation("news");
  const feedback = useFeedback();
  const button = (value: feed.NewsFeedback, label: string, Icon: typeof ThumbsUp, tint: string) => {
    const pressed = article.feedback === value;
    return (
      <button
        type="button"
        aria-pressed={pressed}
        aria-label={compact ? label : undefined}
        title={compact ? label : undefined}
        onClick={() =>
          feedback.mutate({ article, feedback: toggledFeedback(article.feedback, value) })
        }
        className={cn(
          "pressable inline-flex shrink-0 items-center justify-center gap-1.5 rounded-full border-2 font-bold transition-colors",
          compact ? "size-9" : "h-11 min-w-0 flex-1 px-4 text-sm",
          pressed
            ? cn("border-ink shadow-pop-sm", tint)
            : "border-line bg-surface text-ink-soft hover:border-ink-faint hover:text-ink",
        )}
      >
        <Icon className={cn("shrink-0", compact ? "size-4" : "size-[18px]")} />
        {!compact && <span className="truncate">{label}</span>}
      </button>
    );
  };
  return (
    <div className={cn("flex items-center gap-2", className)}>
      {button(feed.NewsFeedback.MORE, t("feedback.more"), ThumbsUp, "bg-lime text-[#17122b]")}
      {button(feed.NewsFeedback.LESS, t("feedback.less"), ThumbsDown, "bg-coral text-[#17122b]")}
    </div>
  );
}

/** An article's lead picture, when this instance serves it (see `SourceIcon`). */
export function ArticlePicture({
  url,
  className,
  lazy = true,
}: {
  url: string;
  className: string;
  lazy?: boolean;
}) {
  const src = linkPreviewImage(url);
  if (!src) return null;
  return (
    <img
      src={src}
      alt=""
      className={className}
      loading={lazy ? "lazy" : undefined}
      decoding="async"
      draggable={false}
    />
  );
}
