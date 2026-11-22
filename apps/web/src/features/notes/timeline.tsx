import { useQueryClient } from "@tanstack/react-query";
import { useElementScrollRestoration } from "@tanstack/react-router";
import { useVirtualizer, type VirtualItem } from "@tanstack/react-virtual";
import type { notes } from "@tokuly/api";
import { ArrowUp } from "lucide-react";
import { type ReactNode, type Ref, useEffect, useImperativeHandle, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { countNewEntries, entryKey, fetchTimeline, notesKeys, type TimelineData, type TimelinePage, type TimelineSource, useDeletedNotes, useTimeline, withFreshEntries } from "./api";
import { onTimelineHint } from "./live";
import { NoteCard } from "./note-card";
import { useScroller } from "./shell";

/** What a page holding a timeline can ask of it. */
export interface TimelineHandle {
  /** Reads the newest notes and puts them on top. */
  refresh(): Promise<void>;
}

export interface TimelineProps {
  source: TimelineSource;
  /**
   * Look for new notes whenever something hints that there are some (see
   * `live.ts`) and when the list comes back on screen. New notes join at
   * once while the reader is at the top, and wait behind a "new notes" pill
   * otherwise.
   */
  live?: boolean;
  /** Shown when the timeline has no notes. */
  empty: ReactNode;
  ref?: Ref<TimelineHandle>;
}

/** Hints closer together than this lead to one look, not several. */
const CHECK_GAP_MS = 2500;
/** A timeline that was off screen for this long looks for new notes when it returns. */
const RETURN_CHECK_MS = 30_000;
/** "At the top": close enough that new notes can simply appear. */
const TOP_ZONE = 120;

export function TimelineSkeleton({ count = 4 }: { count?: number }) {
  return (
    <div className="flex flex-col gap-2.5" aria-busy>
      {Array.from({ length: count }, (_, index) => (
        <div key={index} className="flex gap-3 rounded-[1.25rem] border-2 border-line bg-surface p-3.5">
          <Skeleton className="size-11 shrink-0 rounded-[0.85rem]" />
          <div className="flex flex-1 flex-col gap-2 pt-1">
            <Skeleton className="h-3.5 w-1/3 rounded-full" />
            <Skeleton className="h-3.5 w-11/12 rounded-full" />
            <Skeleton className="h-3.5 w-2/3 rounded-full" />
          </div>
        </div>
      ))}
    </div>
  );
}

/**
 * A timeline of notes: paged as the reader scrolls, only the rows near the
 * screen in the page, counters live, and new notes arriving on top without
 * a reload.
 */
export function Timeline({ source, live = false, empty, ref }: TimelineProps) {
  const { t } = useTranslation("notes");
  const queryClient = useQueryClient();
  const { element: scroller } = useScroller();
  const query = useTimeline(source);
  const deleted = useDeletedNotes((state) => state.ids);
  const [fresh, setFresh] = useState<{ page: TimelinePage; count: number | undefined } | null>(null);
  const key = notesKeys.timeline(source);
  const checking = useRef(false);
  const lastCheck = useRef(0);

  const apply = (page: TimelinePage) => {
    queryClient.setQueryData<TimelineData>(key, (current) => withFreshEntries(current, page));
    setFresh(null);
  };

  const check = async (force: boolean) => {
    if (checking.current || !queryClient.getQueryData(key)) return;
    checking.current = true;
    try {
      const page = await fetchTimeline(source, "");
      lastCheck.current = Date.now();
      const count = countNewEntries(queryClient.getQueryData<TimelineData>(key), page);
      const atTop = (scroller?.scrollTop ?? 0) < TOP_ZONE;
      if (count === 0 || force || atTop) apply(page);
      else setFresh({ page, count });
    } catch (error) {
      if (force) toast.error(errorMessage(error));
    } finally {
      checking.current = false;
    }
  };
  const latestCheck = useRef(check);
  useEffect(() => {
    latestCheck.current = check;
  });

  useImperativeHandle(ref, () => ({
    refresh: async () => {
      if (queryClient.getQueryData(key)) await latestCheck.current(true);
      else await query.refetch();
    },
  }));

  // Hints that something new exists; a burst of them leads to one look.
  useEffect(() => {
    if (!live) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const off = onTimelineHint(() => {
      clearTimeout(timer);
      timer = setTimeout(() => void latestCheck.current(false), Math.max(300, CHECK_GAP_MS - (Date.now() - lastCheck.current)));
    });
    return () => {
      off();
      clearTimeout(timer);
    };
  }, [live]);

  // Coming back to a timeline that was read a while ago: see what was posted meanwhile.
  const dataUpdatedAt = query.dataUpdatedAt;
  // Only when the timeline (re)appears, not on every update of its data.
  useEffect(() => {
    if (dataUpdatedAt > 0 && Date.now() - dataUpdatedAt > RETURN_CHECK_MS) void latestCheck.current(false);
  }, [key.join("|")]);

  const pinned = query.data?.pages[0]?.pinnedNote;
  const seen = new Set<string>();
  const entries = (query.data?.pages.flatMap((page) => page.entries) ?? []).filter((entry) => {
    const id = entry.note?.id;
    if (!id || seen.has(id) || deleted.has(id) || id === pinned?.id) return false;
    seen.add(id);
    return true;
  });

  if (query.isPending) return <TimelineSkeleton />;
  if (query.isError && entries.length === 0) {
    return <EmptyState emoji="🌧️" title={errorMessage(query.error)} action={<Button onClick={() => void query.refetch()}>{t("timeline.retry")}</Button>} />;
  }

  const showPill = fresh !== null;
  return (
    <div data-testid="timeline">
      {showPill && (
        <div className="pointer-events-none sticky z-10 flex h-0 justify-center" style={{ top: "calc(var(--notes-header, 4rem) + 0.6rem)" }}>
          <button
            type="button"
            onClick={() => {
              apply(fresh.page);
              scroller?.scrollTo({ top: 0, behavior: "smooth" });
            }}
            className="sticker pressable pointer-events-auto inline-flex h-10 animate-pop-in items-center gap-1.5 rounded-full bg-brand px-4 text-sm font-black text-on-brand"
          >
            <ArrowUp className="size-4" strokeWidth={3} />
            {fresh.count === undefined ? t("timeline.newMany") : t("timeline.new", { count: fresh.count })}
          </button>
        </div>
      )}
      {pinned && !deleted.has(pinned.id) && <NoteCard note={pinned} showPinned className="mb-2.5" />}
      {entries.length === 0 && !pinned ? empty : <VirtualRows entries={entries} storeKey={key.join("|")} />}
      {entries.length > 0 && <LoadMore query={query} showEnd />}
    </div>
  );
}

/** Measured row heights of timelines that left the screen, so coming back lands on the same note. */
const measurements = new Map<string, { firstKey: string; items: VirtualItem[] }>();
const ROW_GAP = 10;

function VirtualRows({ entries, storeKey }: { entries: notes.TimelineEntry[]; storeKey: string }) {
  // The virtualizer changes without changing identity; the compiler's memoization would freeze the rows.
  "use no memo";
  const { element: scroller, scrollId } = useScroller();
  const list = useRef<HTMLDivElement>(null);
  const [margin, setMargin] = useState(0);
  const keys = entries.map(entryKey);
  const restored = useElementScrollRestoration({ id: scrollId });
  const [saved] = useState(() => {
    const kept = measurements.get(storeKey);
    return kept && kept.firstKey === keys[0] ? kept.items : undefined;
  });

  // Where the list starts inside the scrolling page: whatever sits above it (tabs, stories, a pinned note) may change height.
  useLayoutEffect(() => {
    const element = list.current;
    if (!element || !scroller) return;
    const measure = () => {
      const offset = Math.round(element.getBoundingClientRect().top - scroller.getBoundingClientRect().top + scroller.scrollTop);
      setMargin((current) => (Math.abs(current - offset) > 1 ? offset : current));
    };
    measure();
    const observer = new ResizeObserver(measure);
    for (const child of scroller.children) observer.observe(child);
    return () => observer.disconnect();
  }, [scroller]);

  const virtualizer = useVirtualizer({
    count: entries.length,
    getScrollElement: () => scroller,
    estimateSize: () => 180,
    overscan: 4,
    gap: ROW_GAP,
    scrollMargin: margin,
    getItemKey: (index) => keys[index] ?? index,
    initialOffset: restored?.scrollY,
    initialMeasurementsCache: saved,
  });

  const firstKey = keys[0] ?? "";
  useEffect(
    () => () => {
      measurements.set(storeKey, { firstKey, items: virtualizer.measurementsCache });
    },
    [storeKey, firstKey, virtualizer],
  );

  return (
    <div ref={list} className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
      {virtualizer.getVirtualItems().map((item) => {
        const entry = entries[item.index];
        if (!entry?.note) return null;
        return (
          <div
            key={item.key}
            data-index={item.index}
            ref={virtualizer.measureElement}
            className="absolute top-0 left-0 w-full"
            style={{ transform: `translateY(${item.start - virtualizer.options.scrollMargin}px)` }}
          >
            <NoteCard note={entry.note} renote={entry.renote} />
          </div>
        );
      })}
    </div>
  );
}
