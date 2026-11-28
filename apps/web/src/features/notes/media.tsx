import { Dialog as BaseDialog } from "@base-ui/react/dialog";
import { type common, media, type notes } from "@tokuly/api";
import { ChevronLeft, ChevronRight, Play, Volume2, VolumeX, X } from "lucide-react";
import { type ReactNode, type RefObject, useEffect, useRef, useState } from "react";
import { cn } from "#/lib/cn";
import { loadHls } from "#/lib/hls";
import { useTranslation } from "#/lib/use-translation";

/** The smallest stored rendition that still covers `cssPixels` on this screen. */
export function imageUrl(image: common.Image | undefined, cssPixels: number): string {
  if (!image) return "";
  const wanted =
    cssPixels * Math.min(2, typeof window === "undefined" ? 1 : window.devicePixelRatio || 1);
  const fitting = image.variants
    .filter((variant) => variant.size >= wanted)
    .sort((a, b) => a.size - b.size)[0];
  return fitting?.url ?? image.url;
}

/** The picture of an attachment: the image itself, or a video's poster. */
export function attachmentImage(attachment: media.Attachment): common.Image | undefined {
  return attachment.kind === media.AssetKind.VIDEO ? attachment.video?.poster : attachment.image;
}

export const isVideo = (attachment: media.Attachment): boolean =>
  attachment.kind === media.AssetKind.VIDEO;

/** The person asked for no motion: ドパガキ度 0, or the system setting. */
function motionOff(): boolean {
  return (
    document.documentElement.dataset.dopa === "0" ||
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

/** True while at least `ratio` of the element is on screen. */
export function useOnScreen(target: RefObject<Element | null>, ratio: number): boolean {
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    const element = target.current;
    if (!element) return;
    const observer = new IntersectionObserver(
      ([entry]) => setVisible(Boolean(entry?.isIntersecting && entry.intersectionRatio >= ratio)),
      { threshold: [0, ratio, 1] },
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, [target, ratio]);
  return visible;
}

// ───────────────────────────── Images ─────────────────────────────

const GRID: Record<number, string> = {
  1: "grid-cols-1",
  2: "grid-cols-2 aspect-[16/10]",
  3: "grid-cols-2 grid-rows-2 aspect-[16/10]",
  4: "grid-cols-2 grid-rows-2 aspect-[16/10]",
};
/** Five to ten pictures: rows of three squares. */
const GRID_MANY = "grid-cols-3";

/**
 * Up to four pictures of a note: one keeps its own shape, two sit side by
 * side, three and four share a grid. A tap opens the full-screen viewer.
 */
export function ImageGrid({
  attachments,
  details = [],
  className,
}: {
  attachments: media.Attachment[];
  /** Alt text and sensitive marks by asset ID. */
  details?: notes.AttachmentDetails[];
  className?: string;
}) {
  const { t } = useTranslation("notes");
  const [viewing, setViewing] = useState<number>();
  const [revealed, setRevealed] = useState<Set<string>>(new Set());
  const images = attachments.slice(0, 10);
  const detail = (attachment: media.Attachment) =>
    details.find((item) => item.assetId === attachment.assetId);
  if (images.length === 0) return null;
  const single = images.length === 1 ? images[0]!.image : undefined;
  // A lone picture keeps its proportions, within what a timeline can take.
  const ratio =
    single?.width && single.height
      ? Math.min(2, Math.max(0.75, single.width / single.height))
      : 16 / 10;

  return (
    <>
      <div
        className={cn(
          "grid gap-1 overflow-hidden rounded-2xl border-2 border-ink",
          GRID[images.length] ?? GRID_MANY,
          className,
        )}
        // A tall picture is as wide as its height allows, not letterboxed in a wide box.
        style={
          images.length === 1
            ? { aspectRatio: String(ratio), width: `min(100%, calc(28rem * ${ratio}))` }
            : undefined
        }
      >
        {images.map((attachment, index) => {
          const about = detail(attachment);
          const hidden = about?.sensitive && !revealed.has(attachment.assetId);
          return (
            <button
              key={attachment.assetId || index}
              type="button"
              aria-label={
                hidden
                  ? t("media.revealSensitive")
                  : about?.altText || t("media.open", { index: index + 1, count: images.length })
              }
              onClick={() =>
                hidden ? setRevealed(new Set([...revealed, attachment.assetId])) : setViewing(index)
              }
              className={cn(
                "relative min-h-0 overflow-hidden bg-surface-3",
                images.length === 3 && index === 0 && "row-span-2",
                images.length > 4 && "aspect-square",
              )}
              style={{ background: attachment.image?.dominantColor || undefined }}
            >
              <img
                src={imageUrl(attachment.image, images.length === 1 ? 640 : 320)}
                alt={about?.altText ?? ""}
                title={about?.altText || undefined}
                loading="lazy"
                decoding="async"
                draggable={false}
                className={cn("size-full object-cover", hidden && "scale-110 blur-xl")}
              />
              {hidden && (
                <span className="absolute inset-0 grid place-items-center bg-black/30 text-xs font-black text-white">
                  {t("media.sensitive")}
                </span>
              )}
              {!hidden && about?.altText && (
                <span className="absolute bottom-1 left-1 rounded bg-black/60 px-1 text-[10px] font-black text-white">
                  ALT
                </span>
              )}
            </button>
          );
        })}
      </div>
      {viewing !== undefined && (
        <ImageViewer
          images={images.map((attachment) => attachment.image)}
          start={viewing}
          onClose={() => setViewing(undefined)}
        />
      )}
    </>
  );
}

/** Pictures full screen: swipe or use the arrows to move between them, Escape or the cross to leave. */
export function ImageViewer({
  images,
  start,
  onClose,
}: {
  images: (common.Image | undefined)[];
  start: number;
  onClose(): void;
}) {
  const { t } = useTranslation("notes");
  const strip = useRef<HTMLDivElement>(null);
  const [index, setIndex] = useState(start);

  const go = (next: number, smooth = true) => {
    const element = strip.current;
    if (!element) return;
    const clamped = Math.min(images.length - 1, Math.max(0, next));
    element.scrollTo({
      left: clamped * element.clientWidth,
      behavior: smooth && !motionOff() ? "smooth" : "auto",
    });
    setIndex(clamped);
  };
  // Open on the picture that was tapped.
  useEffect(() => go(start, false), []);

  return (
    <BaseDialog.Root open onOpenChange={(open) => !open && onClose()}>
      <BaseDialog.Portal>
        <BaseDialog.Popup
          aria-label={t("media.viewer")}
          data-testid="image-viewer"
          className="fixed inset-0 z-50 flex flex-col bg-black text-white outline-none transition-opacity duration-150 data-[ending-style]:opacity-0 data-[starting-style]:opacity-0"
          onKeyDown={(event) => {
            if (event.key === "ArrowRight") go(index + 1);
            if (event.key === "ArrowLeft") go(index - 1);
          }}
        >
          <BaseDialog.Title className="sr-only">{t("media.viewer")}</BaseDialog.Title>
          <div className="flex h-14 shrink-0 items-center justify-between px-3 pt-safe">
            <span className="rounded-full bg-white/15 px-3 py-1 text-sm font-black tabular-nums">
              {index + 1} / {images.length}
            </span>
            <BaseDialog.Close
              aria-label={t("media.close")}
              className="grid size-10 place-items-center rounded-full bg-white/15 hover:bg-white/25"
            >
              <X className="size-5" />
            </BaseDialog.Close>
          </div>
          <div
            ref={strip}
            className="scrollbar-none flex min-h-0 flex-1 snap-x snap-mandatory overflow-x-auto"
            onScroll={(event) => {
              const element = event.currentTarget;
              const current = Math.round(element.scrollLeft / Math.max(1, element.clientWidth));
              if (current !== index) setIndex(current);
            }}
          >
            {images.map((image, position) => (
              // A tap beside the picture closes the viewer, like tapping the backdrop of a dialog.
              <div
                key={position}
                className="grid size-full shrink-0 snap-center place-items-center p-2"
                onClick={(event) => event.target === event.currentTarget && onClose()}
              >
                <img
                  src={image?.url}
                  alt=""
                  draggable={false}
                  className="max-h-full max-w-full rounded-lg object-contain"
                />
              </div>
            ))}
          </div>
          {images.length > 1 && (
            <div className="pointer-events-none absolute inset-x-2 top-1/2 hidden -translate-y-1/2 justify-between sm:flex">
              <ViewerArrow
                label={t("media.previous")}
                disabled={index === 0}
                onClick={() => go(index - 1)}
              >
                <ChevronLeft className="size-6" />
              </ViewerArrow>
              <ViewerArrow
                label={t("media.next")}
                disabled={index === images.length - 1}
                onClick={() => go(index + 1)}
              >
                <ChevronRight className="size-6" />
              </ViewerArrow>
            </div>
          )}
          <div className="h-4 shrink-0 pb-safe" />
        </BaseDialog.Popup>
      </BaseDialog.Portal>
    </BaseDialog.Root>
  );
}

function ViewerArrow({
  label,
  disabled,
  onClick,
  children,
}: {
  label: string;
  disabled: boolean;
  onClick(): void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      disabled={disabled}
      onClick={onClick}
      className="pointer-events-auto grid size-11 place-items-center rounded-full bg-white/15 hover:bg-white/25 disabled:opacity-0"
    >
      {children}
    </button>
  );
}

// ───────────────────────────── Video ─────────────────────────────

/**
 * Attaches a video's stream to a `<video>` element: HLS through `hls.js`
 * where the browser needs it, natively where it does not (Safari), the MP4
 * as a last resort. The player library is only fetched once `active`.
 */
export function useVideoSource(
  element: RefObject<HTMLVideoElement | null>,
  video: media.VideoInfo | undefined,
  active: boolean,
): void {
  const hlsUrl = video?.hlsUrl ?? "";
  const mp4Url = video?.mp4Url ?? "";
  useEffect(() => {
    const target = element.current;
    if (!target || !active || (!hlsUrl && !mp4Url)) return;
    if (!hlsUrl || target.canPlayType("application/vnd.apple.mpegurl")) {
      target.src = hlsUrl || mp4Url;
      return;
    }
    let cancelled = false;
    let destroy: (() => void) | undefined;
    // Without the player (or its code), the plain file.
    const playMp4 = () => {
      if (!cancelled) target.src = mp4Url;
    };
    loadHls().then(({ default: Hls }) => {
      if (cancelled) return;
      if (!Hls.isSupported()) return playMp4();
      const hls = new Hls({ capLevelToPlayerSize: true, startLevel: -1 });
      hls.on(Hls.Events.ERROR, (_event, data) => {
        // A stream that cannot be played falls back to the plain file.
        if (data.fatal && mp4Url) {
          hls.destroy();
          target.src = mp4Url;
        }
      });
      hls.loadSource(hlsUrl);
      hls.attachMedia(target);
      destroy = () => hls.destroy();
    }, playMp4);
    return () => {
      cancelled = true;
      destroy?.();
    };
  }, [element, hlsUrl, mp4Url, active]);
}

/**
 * The video of a note: it plays muted by itself while it is on screen
 * (unless motion is off), with a button for the sound and the browser's
 * controls once the person takes over.
 */
export function NoteVideo({
  attachment,
  className,
}: {
  attachment: media.Attachment;
  className?: string;
}) {
  const { t } = useTranslation("notes");
  const box = useRef<HTMLDivElement>(null);
  const element = useRef<HTMLVideoElement>(null);
  const video = attachment.video;
  const onScreen = useOnScreen(box, 0.6);
  // The stream is attached the first time the video comes on screen and kept from then on.
  const [seen, setSeen] = useState(false);
  if (onScreen && !seen) setSeen(true);
  const [muted, setMuted] = useState(true);
  const [takenOver, setTakenOver] = useState(false);
  const [playing, setPlaying] = useState(false);
  useVideoSource(element, video, seen);

  useEffect(() => {
    const target = element.current;
    if (!target || !seen) return;
    if (onScreen && !takenOver && !motionOff()) void target.play().catch(() => {});
    else if (!onScreen) target.pause();
  }, [onScreen, seen, takenOver]);

  const ratio =
    video?.width && video.height
      ? Math.min(16 / 9, Math.max(0.75, video.width / video.height))
      : 16 / 9;
  const poster = imageUrl(video?.poster, 640);
  return (
    <div
      ref={box}
      className={cn("relative overflow-hidden rounded-2xl border-2 border-ink bg-black", className)}
      style={{ aspectRatio: String(ratio), width: `min(100%, calc(30rem * ${ratio}))` }}
      data-testid="note-video"
    >
      <video
        ref={element}
        poster={poster || undefined}
        muted={muted}
        loop
        playsInline
        controls={takenOver}
        preload="none"
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        onClick={() => {
          if (takenOver) return;
          // The first tap hands the video over: sound on, controls shown.
          setTakenOver(true);
          setMuted(false);
          void element.current?.play().catch(() => {});
        }}
        className="size-full object-contain"
      />
      {!playing && !takenOver && (
        <span aria-hidden className="pointer-events-none absolute inset-0 grid place-items-center">
          <span className="grid size-14 place-items-center rounded-full border-2 border-ink bg-white/90 text-[#17122b]">
            <Play className="size-6 translate-x-0.5 fill-current" />
          </span>
        </span>
      )}
      {!takenOver && (
        <button
          type="button"
          aria-label={muted ? t("media.unmute") : t("media.mute")}
          onClick={() => setMuted(!muted)}
          className="absolute right-2 bottom-2 grid size-9 place-items-center rounded-full bg-black/60 text-white hover:bg-black/80"
        >
          {muted ? <VolumeX className="size-4" /> : <Volume2 className="size-4" />}
        </button>
      )}
    </div>
  );
}
