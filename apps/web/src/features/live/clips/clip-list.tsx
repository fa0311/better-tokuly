import { useInfiniteQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { live } from "@tokuly/api";
import { Loader, Scissors, TriangleAlert } from "lucide-react";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { clipClient, liveKeys, nextCursor } from "../api";
import { imageUrl } from "../parts";

const { ClipStatus } = live;

export type ClipFilter = "stream" | "video" | "channel" | "creator";

/** Clips cut from a stream, a video or a channel, or cut by a person; newest first. */
export function useClips(kind: ClipFilter, id: string) {
  return useInfiniteQuery({
    queryKey: liveKeys.clipList(kind, id),
    queryFn: ({ pageParam }) => {
      const page = { cursor: pageParam, limit: 24 };
      const filter =
        kind === "stream"
          ? ({ case: "streamId", value: id } as const)
          : kind === "video"
            ? ({ case: "videoId", value: id } as const)
            : kind === "channel"
              ? ({ case: "channelId", value: id } as const)
              : ({ case: "creatorId", value: id } as const);
      return clipClient().listClips({ filter, page });
    },
    initialPageParam: "",
    getNextPageParam: nextCursor,
    enabled: id !== "",
  });
}

/** A clip as a card that leads to its page. */
export function ClipCard({ clip, className }: { clip: live.Clip; className?: string }) {
  const { t } = useTranslation("live");
  const url = imageUrl(clip.thumbnail, 240);
  return (
    <Link
      to="/clip/$clipId"
      params={{ clipId: clip.shortId }}
      className={cn("group flex min-w-0 flex-col gap-2", className)}
      data-testid="clip-card"
    >
      <span className="relative block aspect-video overflow-hidden rounded-2xl bg-surface-3">
        {url ? (
          <img src={url} alt="" loading="lazy" className="size-full object-cover" />
        ) : (
          <span className="grid size-full place-items-center text-ink-faint">
            {clip.status === ClipStatus.FAILED ? (
              <TriangleAlert className="size-6" />
            ) : clip.status === ClipStatus.PROCESSING ? (
              <Loader className="size-6 animate-spin" />
            ) : (
              <Scissors className="size-6" />
            )}
          </span>
        )}
        <span className="absolute right-1.5 bottom-1.5 rounded-md bg-black/75 px-1.5 py-0.5 text-[11px] font-black text-white tabular-nums">
          {t("clip.seconds", { seconds: clip.durationSeconds.toFixed(1) })}
        </span>
        {clip.status !== ClipStatus.READY && (
          <span className="absolute top-1.5 left-1.5 rounded-md bg-black/75 px-1.5 py-0.5 text-[11px] font-black text-white">
            {t(clip.status === ClipStatus.FAILED ? "clip.failed" : "clip.processing")}
          </span>
        )}
      </span>
      <span className="min-w-0">
        <span className="line-clamp-2 text-sm leading-snug font-black group-hover:text-brand-strong">
          <EmojiText text={clip.title} />
        </span>
        <span className="block truncate text-xs font-bold text-ink-faint">
          {t("clip.by", { name: clip.creator?.displayName ?? "" })}
        </span>
      </span>
    </Link>
  );
}

/** A row of the newest clips of something, shown only when there are any. */
export function ClipStrip({
  kind,
  id,
  className,
}: {
  kind: ClipFilter;
  id: string;
  className?: string;
}) {
  const { t } = useTranslation("live");
  const query = useClips(kind, id);
  const clips = query.data?.pages.flatMap((page) => page.clips) ?? [];
  if (clips.length === 0) return null;
  return (
    <section className={className}>
      <h3 className="flex items-center gap-1.5 text-sm font-black text-ink-soft">
        <Scissors className="size-4" />
        {t("clip.listTitle")}
      </h3>
      <div className="scrollbar-none -mx-1 mt-2 flex gap-3 overflow-x-auto px-1 pb-1">
        {clips.map((clip) => (
          <ClipCard key={clip.id} clip={clip} className="w-44 shrink-0" />
        ))}
      </div>
    </section>
  );
}

/** Every clip of something as a grid, with paging and an empty state. */
export function ClipGrid({ kind, id, mine }: { kind: ClipFilter; id: string; mine?: boolean }) {
  const { t } = useTranslation("live");
  const query = useClips(kind, id);
  const clips = query.data?.pages.flatMap((page) => page.clips) ?? [];
  if (query.isPending)
    return (
      <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-4">
        {Array.from({ length: 4 }, (_, index) => (
          <Skeleton key={index} className="aspect-video w-full" />
        ))}
      </div>
    );
  if (clips.length === 0)
    return (
      <EmptyState
        emoji="✂️"
        title={t(mine ? "clip.emptyMineTitle" : "clip.emptyTitle")}
        body={t(mine ? "clip.emptyMineBody" : "clip.emptyBody")}
      />
    );
  return (
    <>
      <div className="grid grid-cols-2 gap-x-4 gap-y-5 sm:grid-cols-3 lg:grid-cols-4">
        {clips.map((clip) => (
          <ClipCard key={clip.id} clip={clip} />
        ))}
      </div>
      <LoadMore query={query} />
    </>
  );
}
