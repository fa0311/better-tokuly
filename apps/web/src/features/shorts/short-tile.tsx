import { useInfiniteQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { shorts } from "@tokuly/api";
import { Pin, Play } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { compactCount } from "#/features/engagement";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { type ListOptions, listQuery } from "./lists";
import { type FeedContext, imageUrl, searchOfContext } from "./model";

export interface ShortTileProps {
  short: shorts.Short;
  /**
   * The list the tile is shown in: the swipe feed it opens walks through
   * that list. Without one the short opens by itself, followed by the
   * for-you feed.
   */
  context?: FeedContext;
  /** How a search result or a notification says where the viewer came from. */
  via?: "search" | "notification";
  className?: string;
}

/**
 * A short in a grid: its cover in 9:16 with the view count, leading to the
 * swipe feed at that short.
 *
 *   <ShortTile short={short} />
 *   <ShortTile short={short} context={{ kind: "hashtag", key: "ねこ" }} />
 */
export function ShortTile({ short, context, via, className }: ShortTileProps) {
  const { t, i18n } = useTranslation("shorts");
  const cover = imageUrl(short.cover, 180);
  const views = compactCount(Number(short.viewCount), i18n.language);
  return (
    <Link
      to="/shorts/$shortId"
      params={{ shortId: short.shortId }}
      search={context ? searchOfContext(context) : via ? { via } : {}}
      data-testid="short-tile"
      aria-label={t("tile.label", {
        caption: short.caption || short.channel?.name || "",
        views,
      })}
      className={cn(
        "pressable group relative block aspect-[9/16] overflow-hidden rounded-2xl bg-surface-3",
        className,
      )}
      style={short.cover?.dominantColor ? { background: short.cover.dominantColor } : undefined}
    >
      {cover && (
        <img
          src={cover}
          alt=""
          loading="lazy"
          decoding="async"
          className="absolute inset-0 size-full object-cover"
        />
      )}
      <span className="absolute inset-x-0 bottom-0 flex items-end bg-gradient-to-t from-black/75 to-transparent px-2 pt-8 pb-1.5 text-white">
        <span className="min-w-0 flex-1">
          {short.caption && (
            <span className="line-clamp-2 text-xs font-bold leading-snug">{short.caption}</span>
          )}
          <span className="flex items-center gap-1 text-xs font-black tabular-nums">
            <Play className="size-3 fill-current" aria-hidden />
            {views}
          </span>
        </span>
      </span>
      {short.pinned && (
        <span className="absolute top-1.5 left-1.5 flex items-center gap-1 rounded-full bg-sun px-2 py-0.5 text-[11px] font-black text-[#17122b]">
          <Pin className="size-3" aria-hidden />
          {t("tile.pinned")}
        </span>
      )}
    </Link>
  );
}

/** The grid shorts stand in: three across on a phone, more on wide screens. */
export function ShortGrid({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div
      className={cn(
        "grid grid-cols-3 gap-1.5 sm:grid-cols-4 sm:gap-2.5 lg:grid-cols-5 xl:grid-cols-6",
        className,
      )}
    >
      {children}
    </div>
  );
}

export function ShortGridSkeleton({ count = 9 }: { count?: number }) {
  return (
    <ShortGrid>
      {Array.from({ length: count }, (_, index) => (
        <Skeleton key={index} className="aspect-[9/16]" />
      ))}
    </ShortGrid>
  );
}

/**
 * A whole list of shorts as a grid, with loading, paging and an empty state:
 * a channel's shorts, a hashtag's, a sound's, the saved ones.
 */
export function ShortList({
  context,
  options,
  empty,
}: {
  context: FeedContext;
  options?: ListOptions;
  /** What to show when the list has nothing. */
  empty: ReactNode;
}) {
  const query = useInfiniteQuery(listQuery(context, options));
  if (query.isPending) return <ShortGridSkeleton />;
  if (query.isError)
    return <EmptyState emoji="😵‍💫" title={errorMessage(query.error, { namespace: "shorts" })} />;
  const entries = query.data.pages.flatMap((page) => page.entries);
  if (entries.length === 0) return empty;
  return (
    <>
      <ShortGrid>
        {entries.map(({ short }) => (
          <ShortTile key={short.id} short={short} context={context} />
        ))}
      </ShortGrid>
      <LoadMore query={query} />
    </>
  );
}
