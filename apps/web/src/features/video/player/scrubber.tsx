import type { video } from "@tokuly/api";
import { chapterAt } from "@tokuly/core/video";
import {
  type PointerEvent as ReactPointerEvent,
  type RefObject,
  useEffect,
  useRef,
  useState,
} from "react";
import { cn } from "#/lib/cn";
import { formatClock } from "../format";
import { scriptUrl } from "./media-url";
import { frameAt, parseStoryboard, type StoryboardFrame } from "./vtt";

/** Loads the seek-preview track the first time it is wanted. */
function useStoryboard(url: string, wanted: boolean): StoryboardFrame[] {
  const [frames, setFrames] = useState<StoryboardFrame[]>([]);
  useEffect(() => {
    if (!wanted || !url) return;
    const abort = new AbortController();
    const trackUrl = new URL(scriptUrl(url), window.location.origin).href;
    fetch(trackUrl, { signal: abort.signal })
      .then((response) => (response.ok ? response.text() : ""))
      .then((text) => setFrames(parseStoryboard(text, trackUrl)))
      .catch(() => {
        // No previews; the time label still shows.
      });
    return () => abort.abort();
  }, [url, wanted]);
  return frames;
}

export interface ScrubberProps {
  media: RefObject<HTMLVideoElement | null>;
  time: number;
  duration: number;
  bufferedEnd: number;
  chapters: video.Chapter[];
  storyboardUrl: string;
  label: string;
  disabled?: boolean;
  onSeek(seconds: number): void;
  /** Dragging started / ended: the controls stay up meanwhile. */
  onScrubbing?(active: boolean): void;
}

/**
 * The progress bar: played and buffered parts, gaps at chapter starts, and a
 * preview (frame, chapter, time) over the position under the pointer. It is
 * dragged with the mouse or a finger; the keyboard moves it through the
 * player's shortcuts.
 */
export function Scrubber({
  media,
  time,
  duration,
  bufferedEnd,
  chapters,
  storyboardUrl,
  label,
  disabled,
  onSeek,
  onScrubbing,
}: ScrubberProps) {
  const track = useRef<HTMLDivElement>(null);
  const fill = useRef<HTMLDivElement>(null);
  const thumb = useRef<HTMLDivElement>(null);
  const [hover, setHover] = useState<{ x: number; width: number; time: number } | null>(null);
  const [dragging, setDragging] = useState(false);
  const frames = useStoryboard(storyboardUrl, hover !== null);
  const length = Math.max(duration, 0.001);

  // Between the element's own time updates (a few per second) the bar keeps moving every frame.
  const dragTime = dragging ? hover?.time : undefined;
  useEffect(() => {
    let frame = 0;
    const paint = () => {
      const position = dragTime ?? media.current?.currentTime ?? 0;
      const percent = `${Math.min(100, Math.max(0, (position / length) * 100))}%`;
      if (fill.current) fill.current.style.width = percent;
      if (thumb.current) thumb.current.style.left = percent;
      frame = requestAnimationFrame(paint);
    };
    paint();
    return () => cancelAnimationFrame(frame);
  }, [media, length, dragTime]);

  const locate = (clientX: number) => {
    const box = track.current?.getBoundingClientRect();
    if (!box || box.width === 0) return null;
    const x = Math.min(box.width, Math.max(0, clientX - box.left));
    return { x, width: box.width, time: (x / box.width) * length };
  };

  const onPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (disabled || (event.pointerType === "mouse" && event.button !== 0)) return;
    const at = locate(event.clientX);
    if (!at) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    setDragging(true);
    setHover(at);
    onScrubbing?.(true);
  };
  const onPointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (disabled) return;
    // A resting finger is not hovering.
    if (event.pointerType !== "mouse" && !dragging) return;
    const at = locate(event.clientX);
    if (at) setHover(at);
  };
  const finish = (event: ReactPointerEvent<HTMLDivElement>, commit: boolean) => {
    if (!dragging) return;
    const at = locate(event.clientX) ?? hover;
    setDragging(false);
    onScrubbing?.(false);
    if (commit && at) onSeek(at.time);
    if (event.pointerType !== "mouse") setHover(null);
  };

  const frame = hover ? frameAt(frames, hover.time) : undefined;
  const hoverChapter = hover ? chapterAt(chapters, hover.time) : undefined;
  // The preview stays inside the bar's ends.
  const previewWidth = frame ? frame.width : 64;
  const previewLeft = hover
    ? Math.min(
        Math.max(hover.x, previewWidth / 2),
        Math.max(previewWidth / 2, hover.width - previewWidth / 2),
      )
    : 0;
  const starts = chapters
    .map((chapter) => chapter.startSeconds)
    .filter((start) => start > 0.5 && start < length - 0.5);

  return (
    <div
      ref={track}
      role="slider"
      tabIndex={disabled ? -1 : 0}
      data-player-scrubber
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={Math.round(length)}
      aria-valuenow={Math.round(time)}
      aria-valuetext={`${formatClock(time)} / ${formatClock(length)}`}
      aria-disabled={disabled || undefined}
      className={cn(
        "group/scrub relative flex h-5 touch-none items-center select-none",
        disabled ? "cursor-default" : "cursor-pointer",
      )}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={(event) => finish(event, true)}
      onPointerCancel={(event) => finish(event, false)}
      onPointerLeave={(event) => !dragging && event.pointerType === "mouse" && setHover(null)}
    >
      <div
        className={cn(
          "relative h-1 w-full rounded-full bg-white/30 transition-[height] duration-100",
          !disabled && "group-hover/scrub:h-1.5",
          dragging && "h-1.5",
        )}
      >
        <div
          className="absolute inset-y-0 left-0 rounded-full bg-white/45"
          style={{ width: `${Math.min(100, (bufferedEnd / length) * 100)}%` }}
        />
        <div ref={fill} className="absolute inset-y-0 left-0 rounded-full bg-brand" />
        {starts.map((start) => (
          <span
            key={start}
            aria-hidden
            className="absolute inset-y-0 w-0.5 -translate-x-1/2 bg-black/70"
            style={{ left: `${(start / length) * 100}%` }}
          />
        ))}
      </div>
      {!disabled && (
        <div
          ref={thumb}
          aria-hidden
          className={cn(
            "pointer-events-none absolute top-1/2 size-3.5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-white bg-brand transition-transform duration-100",
            dragging
              ? "scale-125"
              : "scale-0 group-hover/scrub:scale-100 group-focus-visible/scrub:scale-100",
          )}
        />
      )}
      {hover && !disabled && (
        <div
          aria-hidden
          className="pointer-events-none absolute bottom-full mb-2 flex -translate-x-1/2 flex-col items-center gap-1"
          style={{ left: previewLeft }}
        >
          {frame && (
            <div
              className="rounded-lg border-2 border-white bg-black bg-no-repeat"
              style={{
                width: frame.width,
                height: frame.height,
                backgroundImage: `url("${frame.url}")`,
                backgroundPosition: `-${frame.x}px -${frame.y}px`,
              }}
            />
          )}
          {hoverChapter && (
            <span className="max-w-48 truncate rounded-md bg-black/80 px-1.5 py-0.5 text-[11px] font-bold text-white">
              {hoverChapter.title}
            </span>
          )}
          <span className="rounded-md bg-black/80 px-1.5 py-0.5 text-xs font-black text-white tabular-nums">
            {formatClock(hover.time)}
          </span>
        </div>
      )}
    </div>
  );
}
