import { Link } from "@tanstack/react-router";
import type { feed } from "@tokuly/api";
import { cn } from "#/lib/cn";
import { FeedbackButtons, PublishedAt, RandomPickChip, SourceIcon } from "./parts";

/**
 * An article in the news list. The whole card opens the article's page; the
 * two feedback buttons sit on top of it. A row on phones (picture on the
 * right), a tile with the picture on top where there is room.
 */
export function ArticleCard({
  article,
  className,
}: {
  article: feed.NewsArticle;
  className?: string;
}) {
  return (
    <article
      data-testid="news-article"
      className={cn(
        "sticker pressable relative flex h-full gap-3 overflow-hidden rounded-card bg-surface p-3 md:flex-col md:gap-0 md:p-0",
        className,
      )}
    >
      {article.imageUrl && (
        <img
          src={article.imageUrl}
          alt=""
          className="order-last size-24 shrink-0 rounded-xl bg-surface-3 object-cover md:order-first md:aspect-video md:h-auto md:w-full md:rounded-none md:border-b-[2.5px] md:border-ink"
          loading="lazy"
          decoding="async"
          draggable={false}
        />
      )}
      <div className="flex min-w-0 flex-1 flex-col gap-1.5 md:p-4">
        <p className="flex min-w-0 items-center gap-1.5 text-xs text-ink-soft">
          <SourceIcon source={article.source} />
          <span className="truncate font-bold">{article.source?.name}</span>
          <span aria-hidden>·</span>
          <PublishedAt at={article.publishedAt} className="shrink-0" />
        </p>
        <h3
          className={cn(
            "line-clamp-3 leading-snug font-black break-words",
            article.clicked && "text-ink-soft",
          )}
        >
          {/* The link covers the card; controls above it stay clickable. */}
          <Link
            to="/news/$articleId"
            params={{ articleId: article.id }}
            className="after:absolute after:inset-0 after:rounded-card"
          >
            {article.title}
          </Link>
        </h3>
        {article.summary && (
          <p className="line-clamp-2 hidden text-sm leading-snug break-words text-ink-soft md:[display:-webkit-box]">
            {article.summary}
          </p>
        )}
        <div className="mt-auto flex min-w-0 items-center gap-2 pt-1">
          {article.randomPick && <RandomPickChip />}
          {article.tags.length > 0 && (
            <span className="min-w-0 flex-1 truncate text-xs font-bold text-ink-faint">
              {article.tags
                .slice(0, 3)
                .map((tag) => `#${tag}`)
                .join(" ")}
            </span>
          )}
          <FeedbackButtons article={article} compact className="relative z-10 ml-auto" />
        </div>
      </div>
    </article>
  );
}

/** The outline of a card while the list loads. */
export function ArticleSkeleton() {
  return (
    <div
      className="flex h-32 gap-3 rounded-card border-2 border-line bg-surface p-3 md:h-72 md:flex-col"
      aria-hidden
    >
      <div className="order-last size-24 shrink-0 animate-pulse rounded-xl bg-surface-3 md:order-first md:h-32 md:w-full" />
      <div className="flex min-w-0 flex-1 flex-col gap-2 py-1">
        <div className="h-3 w-1/3 animate-pulse rounded-full bg-surface-3" />
        <div className="h-4 w-full animate-pulse rounded-full bg-surface-3" />
        <div className="h-4 w-3/4 animate-pulse rounded-full bg-surface-3" />
      </div>
    </div>
  );
}
