import { type common, media } from "@tokuly/api";
import { waveformBars } from "@tokuly/core/chat";
import { CircleAlert, Download, FileText, Pause, Play } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Spinner } from "#/components/ui/spinner";
import { cn } from "#/lib/cn";
import { fileSize, playTime } from "./text";

/** How attachments are drawn inside messages, the gallery and the composer. */

export function isVisual(attachment: media.Attachment): boolean {
  return (
    attachment.kind === media.AssetKind.IMAGE ||
    attachment.kind === media.AssetKind.ANIMATION ||
    attachment.kind === media.AssetKind.VIDEO
  );
}

/** The still picture of an attachment: the image itself, or a video's poster. */
export function pictureOf(attachment: media.Attachment): common.Image | undefined {
  return attachment.kind === media.AssetKind.VIDEO ? attachment.video?.poster : attachment.image;
}

/** Smallest stored rendition that still covers `pixels` on a 2x screen. */
export function pictureUrl(image: common.Image | undefined, pixels: number): string {
  if (!image) return "";
  const fitting = image.variants
    .filter((variant) => variant.size >= pixels * 2)
    .sort((a, b) => a.size - b.size)[0];
  return fitting?.url ?? image.url;
}

/**
 * One picture or video as a tile. An animation plays in place; a video shows
 * its poster with a play mark (and says so while the server is still
 * processing it). `pixels` is the longest edge it is drawn at.
 */
export function VisualTile({
  attachment,
  pixels,
  className,
  onOpen,
  label,
}: {
  attachment: media.Attachment;
  pixels: number;
  className?: string;
  onOpen?(): void;
  label: string;
}) {
  const { t } = useTranslation("chat");
  const video = attachment.kind === media.AssetKind.VIDEO;
  const picture = pictureOf(attachment);
  // An animation is its own moving picture; a still loads the rendition that fits.
  const source =
    attachment.kind === media.AssetKind.ANIMATION
      ? attachment.url || pictureUrl(picture, pixels)
      : pictureUrl(picture, pixels) || (video ? "" : attachment.url);
  const processing =
    attachment.status === media.AssetStatus.PROCESSING ||
    attachment.status === media.AssetStatus.UPLOADING;
  const failed = attachment.status === media.AssetStatus.FAILED;
  const localVideo = video && !source && attachment.video?.mp4Url;

  return (
    <button
      type="button"
      onClick={onOpen}
      disabled={!onOpen || failed}
      aria-label={label}
      className={cn("relative block overflow-hidden bg-surface-3", className)}
      style={picture?.dominantColor ? { background: picture.dominantColor } : undefined}
    >
      {source ? (
        <img
          src={source}
          alt=""
          className="size-full object-cover"
          loading="lazy"
          decoding="async"
          draggable={false}
        />
      ) : localVideo ? (
        // A video that is still on its way up: its first frame stands in for the poster.
        <video
          src={`${attachment.video!.mp4Url}#t=0.1`}
          muted
          playsInline
          preload="metadata"
          className="size-full object-cover"
        />
      ) : null}
      {video && !failed && (
        <span className="absolute inset-0 grid place-items-center bg-black/15">
          {processing && !localVideo ? (
            <span className="flex items-center gap-2 rounded-full bg-black/60 px-3 py-1.5 text-xs font-bold text-white">
              <Spinner className="size-4" />
              {t("media.processing")}
            </span>
          ) : (
            <span className="grid size-12 place-items-center rounded-full bg-black/55 text-white">
              <Play className="size-6 translate-x-0.5 fill-current" />
            </span>
          )}
        </span>
      )}
      {video && (attachment.video?.durationSeconds ?? 0) > 0 && (
        <span className="absolute right-1.5 bottom-1.5 rounded-md bg-black/60 px-1.5 py-0.5 text-[11px] font-bold text-white tabular-nums">
          {playTime(attachment.video!.durationSeconds)}
        </span>
      )}
      {failed && (
        <span className="absolute inset-0 grid place-items-center gap-1 bg-surface-3 p-2 text-center text-xs font-bold text-ink-soft">
          <CircleAlert className="mx-auto size-6" />
          {t("media.failed")}
        </span>
      )}
    </button>
  );
}

/** Only one voice message plays at a time: starting one pauses the other. */
let playing: HTMLAudioElement | null = null;

/** A voice message: play/pause, the waveform as a seek bar, and the time. */
export function VoicePlayer({
  attachment,
  tone,
}: {
  attachment: media.Attachment /** On the viewer's own (brand coloured) bubble. */;
  tone: "own" | "other";
}) {
  const { t } = useTranslation("chat");
  const audio = useRef<HTMLAudioElement>(null);
  const [isPlaying, setPlaying] = useState(false);
  const [position, setPosition] = useState(0);
  const [measured, setMeasured] = useState(0);
  const duration = attachment.video?.durationSeconds || measured;
  const bars = waveformBars(attachment.waveform);
  const progress = duration > 0 ? Math.min(1, position / duration) : 0;

  useEffect(
    () => () => {
      if (playing === audio.current) playing = null;
    },
    [],
  );

  const toggle = () => {
    const element = audio.current;
    if (!element) return;
    if (element.paused) {
      if (playing && playing !== element) playing.pause();
      playing = element;
      void element.play().catch(() => setPlaying(false));
    } else {
      element.pause();
    }
  };

  const seek = (clientX: number, target: HTMLElement) => {
    const element = audio.current;
    if (!element || !duration) return;
    const box = target.getBoundingClientRect();
    element.currentTime = Math.min(1, Math.max(0, (clientX - box.left) / box.width)) * duration;
    setPosition(element.currentTime);
  };

  const own = tone === "own";
  return (
    <div className="flex w-56 max-w-full items-center gap-2.5" data-testid="voice-message">
      <button
        type="button"
        onClick={toggle}
        aria-label={isPlaying ? t("media.pause") : t("media.play")}
        className={cn(
          "grid size-10 shrink-0 place-items-center rounded-full",
          own ? "bg-on-brand text-brand" : "bg-brand text-on-brand",
        )}
      >
        {isPlaying ? (
          <Pause className="size-5 fill-current" />
        ) : (
          <Play className="size-5 translate-x-px fill-current" />
        )}
      </button>
      <div className="min-w-0 flex-1">
        <div
          role="slider"
          tabIndex={0}
          aria-label={t("media.seek")}
          aria-valuemin={0}
          aria-valuemax={Math.round(duration)}
          aria-valuenow={Math.round(position)}
          onClick={(event) => seek(event.clientX, event.currentTarget)}
          onKeyDown={(event) => {
            const element = audio.current;
            if (!element || (event.key !== "ArrowLeft" && event.key !== "ArrowRight")) return;
            element.currentTime = Math.min(
              duration,
              Math.max(0, element.currentTime + (event.key === "ArrowRight" ? 2 : -2)),
            );
          }}
          className="flex h-8 cursor-pointer items-center gap-[2px]"
        >
          {bars.map((height, index) => (
            <span
              key={index}
              className={cn(
                "min-w-0 flex-1 rounded-full",
                index / bars.length < progress
                  ? own
                    ? "bg-on-brand"
                    : "bg-brand"
                  : own
                    ? "bg-on-brand/40"
                    : "bg-ink-faint/50",
              )}
              style={{ height: `${Math.round(height * 100)}%` }}
            />
          ))}
        </div>
        <p
          className={cn(
            "text-[11px] font-bold tabular-nums",
            own ? "text-on-brand/80" : "text-ink-soft",
          )}
        >
          {playTime(isPlaying || position > 0 ? position : duration)}
        </p>
      </div>
      <audio
        ref={audio}
        src={attachment.url}
        preload="metadata"
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        onEnded={() => {
          setPlaying(false);
          setPosition(0);
        }}
        onTimeUpdate={(event) => setPosition(event.currentTarget.currentTime)}
        onLoadedMetadata={(event) =>
          Number.isFinite(event.currentTarget.duration) && setMeasured(event.currentTarget.duration)
        }
      />
    </div>
  );
}

/** A file to download: its name, its size, and where it leads. */
export function FileRow({
  attachment,
  tone,
}: {
  attachment: media.Attachment;
  tone: "own" | "other" | "plain";
}) {
  const { t } = useTranslation("chat");
  const own = tone === "own";
  const name = attachment.fileName || t("preview.file");
  return (
    <a
      href={attachment.url || undefined}
      download={name}
      target="_blank"
      rel="noreferrer"
      className={cn(
        "flex w-64 max-w-full items-center gap-3 rounded-xl p-1 text-left",
        tone === "plain" && "w-full rounded-2xl px-3 py-2.5 hover:bg-surface-2",
      )}
    >
      <span
        className={cn(
          "grid size-10 shrink-0 place-items-center rounded-xl",
          own ? "bg-on-brand/20" : "bg-surface-2 text-ink-soft",
        )}
      >
        <FileText className="size-5" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-bold">{name}</span>
        <span className={cn("block text-xs", own ? "text-on-brand/75" : "text-ink-soft")}>
          {fileSize(Number(attachment.sizeBytes))}
        </span>
      </span>
      <Download
        className={cn("size-5 shrink-0", own ? "text-on-brand/80" : "text-ink-faint")}
        aria-label={t("media.download")}
      />
    </a>
  );
}
