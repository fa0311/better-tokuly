import type { shorts } from "@tokuly/api";
import { ChevronDown, ChevronUp } from "lucide-react";
import { type ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "#/lib/cn";
import type { FeedEntry } from "../lists";
import { slideLoad } from "../model";
import { ShortSlide } from "./slide";
import { setFeedMuted, useFeedSound } from "./use-short-video";

/** How long the wheel is ignored after it turned a page: one flick of a trackpad is one short. */
const WHEEL_LOCK_MS = 650;

function isTyping(target: EventTarget | null): boolean {
  return (
    target instanceof Element &&
    target.closest("input, textarea, select, [contenteditable='true']") !== null
  );
}

/**
 * The swipe feed's scroller: one short per page, snapping. Only the short
 * on screen plays; it and the next one hold a `<video>`, everything further
 * away is an empty box, so a long feed costs no more than a short one.
 *
 * Touch scrolls natively. The wheel turns one page per flick; the arrow
 * keys, Page Up/Down and J/K turn pages, Space pauses, M mutes.
 */
export function ShortPager({
  entries,
  source,
  startIndex,
  onIndexChange,
  end,
}: {
  entries: FeedEntry[];
  source: shorts.PlaybackSource;
  /** The entry to open on. Read once. */
  startIndex: number;
  /** The entry on screen changed (`entries.length` = the end slide). */
  onIndexChange(index: number): void;
  /** The slide after the last short: "loading more", "that was all", an invitation. */
  end: ReactNode;
}) {
  const { t } = useTranslation("shorts");
  const scroller = useRef<HTMLDivElement>(null);
  const [current, setCurrent] = useState(startIndex);
  const count = entries.length;

  // Open on the right short before the first paint.
  const first = useRef(startIndex);
  useLayoutEffect(() => {
    const element = scroller.current;
    if (element && first.current > 0) element.scrollTop = first.current * element.clientHeight;
  }, []);

  // Which slide is on screen: the one most of the viewport shows. An observer, so the swipe itself does no layout work.
  useEffect(() => {
    const element = scroller.current;
    if (!element) return;
    const observer = new IntersectionObserver(
      (observed) => {
        for (const entry of observed) {
          if (!entry.isIntersecting) continue;
          setCurrent(Number((entry.target as HTMLElement).dataset.index));
        }
      },
      { root: element, threshold: 0.6 },
    );
    for (const child of element.children) observer.observe(child);
    return () => observer.disconnect();
    // The end slide is one more child.
  }, [count]);

  useEffect(() => onIndexChange(current), [current, onIndexChange]);

  const go = (delta: number) => {
    const element = scroller.current;
    const target = element?.children[Math.min(Math.max(0, current + delta), count)];
    if (!target) return;
    const calm = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    target.scrollIntoView({ behavior: calm ? "auto" : "smooth", block: "start" });
  };
  const togglePlayback = () =>
    scroller.current?.children[current]?.querySelector<HTMLElement>("[data-surface]")?.click();
  const goRef = useRef(go);
  const toggleRef = useRef(togglePlayback);
  useEffect(() => {
    goRef.current = go;
    toggleRef.current = togglePlayback;
  });

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.metaKey || event.ctrlKey || event.altKey) return;
      // A sheet or a menu is open, or someone is typing: the keys are theirs.
      if (isTyping(event.target) || document.querySelector("[role='dialog'], [role='menu']"))
        return;
      if (event.key === "ArrowDown" || event.key === "PageDown" || event.key === "j") {
        event.preventDefault();
        goRef.current(1);
      } else if (event.key === "ArrowUp" || event.key === "PageUp" || event.key === "k") {
        event.preventDefault();
        goRef.current(-1);
      } else if (
        event.key === " " &&
        !(event.target as Element).closest("button, a, [role='tab']")
      ) {
        event.preventDefault();
        toggleRef.current();
      } else if (event.key === "m") {
        const sound = useFeedSound.getState();
        setFeedMuted(!(sound.muted || sound.blocked));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // A mouse wheel moves a snapping scroller by a notch and it springs back: turn a page instead.
  useEffect(() => {
    const element = scroller.current;
    if (!element) return;
    let lockedUntil = 0;
    const onWheel = (event: WheelEvent) => {
      if (event.ctrlKey || Math.abs(event.deltaY) < Math.abs(event.deltaX)) return;
      event.preventDefault();
      if (event.timeStamp < lockedUntil || Math.abs(event.deltaY) < 8) return;
      lockedUntil = event.timeStamp + WHEEL_LOCK_MS;
      goRef.current(event.deltaY > 0 ? 1 : -1);
    };
    element.addEventListener("wheel", onWheel, { passive: false });
    return () => element.removeEventListener("wheel", onWheel);
  }, []);

  const seen = new Map<string, number>();
  return (
    <>
      <div
        ref={scroller}
        data-testid="short-pager"
        className="scrollbar-none absolute inset-0 snap-y snap-mandatory overflow-y-auto overscroll-y-contain bg-black md:rounded-[2rem] md:border-[3px] md:border-ink"
      >
        {entries.map((entry, index) => {
          // The for-you feed may bring a short round again: the key tells the visits apart.
          const visit = seen.get(entry.short.id) ?? 0;
          seen.set(entry.short.id, visit + 1);
          const load = slideLoad(index, current);
          return (
            <div
              key={`${entry.short.id}:${visit}`}
              data-index={index}
              className="h-full w-full snap-start snap-always"
            >
              {load !== "empty" && <ShortSlide entry={entry} load={load} source={source} />}
            </div>
          );
        })}
        <div data-index={count} className="h-full w-full snap-start snap-always">
          {end}
        </div>
      </div>
      <PagerArrows
        className="hidden md:flex"
        previous={current > 0 ? () => go(-1) : undefined}
        next={current < count ? () => go(1) : undefined}
        labels={{ previous: t("pager.previous"), next: t("pager.next") }}
      />
    </>
  );
}

/** Up and down buttons next to the stage, for a mouse. */
function PagerArrows({
  previous,
  next,
  labels,
  className,
}: {
  previous?: () => void;
  next?: () => void;
  labels: { previous: string; next: string };
  className?: string;
}) {
  const button =
    "pressable sticker grid size-12 place-items-center rounded-full bg-surface text-ink disabled:opacity-40";
  return (
    <div
      className={cn("absolute top-1/2 left-full ml-4 -translate-y-1/2 flex-col gap-3", className)}
    >
      <button
        type="button"
        className={button}
        aria-label={labels.previous}
        disabled={!previous}
        onClick={previous}
      >
        <ChevronUp className="size-6" />
      </button>
      <button
        type="button"
        className={button}
        aria-label={labels.next}
        disabled={!next}
        onClick={next}
      >
        <ChevronDown className="size-6" />
      </button>
    </div>
  );
}
