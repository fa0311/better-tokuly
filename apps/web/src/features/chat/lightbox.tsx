import { Dialog as BaseDialog } from "@base-ui/react/dialog";
import { media } from "@tokuly/api";
import { ChevronLeft, ChevronRight, Download, MessageSquareShare, X } from "lucide-react";
import { type PointerEvent, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "#/lib/cn";
import { pictureOf } from "./media-parts";

/** One picture or video shown full screen. */
export interface LightboxItem {
  key: string;
  attachment: media.Attachment;
  /** Line under the picture: who sent it and when, or the caption. */
  caption?: string;
  /** The message it belongs to, for "show in chat". */
  messageId?: string;
}

/**
 * Photos and videos full screen: swipe or use the arrow keys to move between
 * them, double-tap a photo to zoom, save it, or jump to its message.
 */
export function Lightbox({ items, index, onIndexChange, onClose, onShowInChat }: { items: LightboxItem[]; index: number | null; onIndexChange(index: number): void; onClose(): void; onShowInChat?(messageId: string): void }) {
  const { t } = useTranslation("chat");
  const open = index !== null && items.length > 0;
  // The last item stays up while the viewer fades out.
  const [last, setLast] = useState(index ?? 0);
  if (index !== null && index !== last) setLast(index);
  const current = Math.min(last, Math.max(0, items.length - 1));
  const item = items[current];
  const [zoomed, setZoomed] = useState(false);
  const drag = useRef<{ x: number; y: number } | null>(null);

  const go = (step: number) => {
    const next = current + step;
    if (next < 0 || next >= items.length) return;
    setZoomed(false);
    onIndexChange(next);
  };

  useEffect(() => {
    if (!open) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "ArrowLeft") go(-1);
      if (event.key === "ArrowRight") go(1);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const onPointerDown = (event: PointerEvent) => {
    drag.current = { x: event.clientX, y: event.clientY };
  };
  const onPointerUp = (event: PointerEvent) => {
    const start = drag.current;
    drag.current = null;
    if (!start || zoomed) return;
    const dx = event.clientX - start.x;
    const dy = event.clientY - start.y;
    if (Math.abs(dx) > 60 && Math.abs(dx) > Math.abs(dy) * 1.5) go(dx < 0 ? 1 : -1);
    else if (dy > 110 && Math.abs(dy) > Math.abs(dx) * 1.5) onClose();
  };

  if (!item) return null;
  const video = item.attachment.kind === media.AssetKind.VIDEO;
  const downloadUrl = video ? item.attachment.video?.mp4Url || item.attachment.url : item.attachment.url || pictureOf(item.attachment)?.url;
  const round = "grid size-11 place-items-center rounded-full bg-white/15 text-white backdrop-blur transition-colors hover:bg-white/25";

  return (
    <BaseDialog.Root open={open} onOpenChange={(next) => !next && onClose()}>
      <BaseDialog.Portal>
        <BaseDialog.Popup
          aria-label={t("media.viewer")}
          data-testid="lightbox"
          className="fixed inset-0 z-[70] flex flex-col bg-black/95 text-white outline-none transition-opacity duration-200 data-[ending-style]:opacity-0 data-[starting-style]:opacity-0"
        >
          <BaseDialog.Title className="sr-only">{t("media.viewer")}</BaseDialog.Title>
          <header className="flex shrink-0 items-center gap-2 px-3 pt-safe pb-2">
            <BaseDialog.Close aria-label={t("actions.close")} className={round}>
              <X className="size-5" />
            </BaseDialog.Close>
            <p className="min-w-0 flex-1 text-center text-sm font-bold tabular-nums">{items.length > 1 ? `${current + 1} / ${items.length}` : ""}</p>
            {item.messageId && onShowInChat && (
              <button
                type="button"
                aria-label={t("media.showInChat")}
                title={t("media.showInChat")}
                className={round}
                onClick={() => {
                  onClose();
                  onShowInChat(item.messageId!);
                }}
              >
                <MessageSquareShare className="size-5" />
              </button>
            )}
            {downloadUrl && (
              <a href={downloadUrl} download={item.attachment.fileName || true} target="_blank" rel="noreferrer" aria-label={t("media.save")} title={t("media.save")} className={round}>
                <Download className="size-5" />
              </a>
            )}
          </header>

          <div className="relative min-h-0 flex-1 touch-none select-none overflow-hidden" onPointerDown={onPointerDown} onPointerUp={onPointerUp}>
            {video ? (
              <LightboxVideo key={item.key} attachment={item.attachment} />
            ) : (
              <img
                key={item.key}
                src={item.attachment.url || pictureOf(item.attachment)?.url}
                alt={item.caption ?? ""}
                draggable={false}
                onDoubleClick={() => setZoomed((value) => !value)}
                className={cn("size-full object-contain transition-transform duration-200", zoomed ? "scale-[2.2] cursor-zoom-out" : "cursor-zoom-in")}
              />
            )}
            {current > 0 && (
              <button type="button" aria-label={t("media.previous")} onClick={() => go(-1)} className={cn(round, "absolute top-1/2 left-3 hidden -translate-y-1/2 md:grid")}>
                <ChevronLeft className="size-6" />
              </button>
            )}
            {current < items.length - 1 && (
              <button type="button" aria-label={t("media.next")} onClick={() => go(1)} className={cn(round, "absolute top-1/2 right-3 hidden -translate-y-1/2 md:grid")}>
                <ChevronRight className="size-6" />
              </button>
            )}
          </div>

          <footer className="min-h-12 shrink-0 px-4 pt-2 pb-safe text-center text-sm text-white/85">{item.caption}</footer>
        </BaseDialog.Popup>
      </BaseDialog.Portal>
    </BaseDialog.Root>
  );
}

/** A video with the browser's controls. Short clips play as MP4; longer ones stream (HLS). */
function LightboxVideo({ attachment }: { attachment: media.Attachment }) {
  const element = useRef<HTMLVideoElement>(null);
  const info = attachment.video;
  const direct = info?.mp4Url || (attachment.url && !attachment.url.includes(".m3u8") ? attachment.url : "");
  const stream = direct ? "" : info?.hlsUrl || attachment.url;

  useEffect(() => {
    const video = element.current;
    if (!video || !stream) return;
    if (video.canPlayType("application/vnd.apple.mpegurl")) {
      video.src = stream;
      return;
    }
    let cancelled = false;
    let player: { destroy(): void } | undefined;
    void import("hls.js").then(({ default: Hls }) => {
      if (cancelled || !Hls.isSupported()) return;
      const hls = new Hls();
      hls.loadSource(stream);
      hls.attachMedia(video);
      player = hls;
    });
    return () => {
      cancelled = true;
      player?.destroy();
    };
  }, [stream]);

  return (
    <video ref={element} src={direct || undefined} poster={info?.poster?.url} controls autoPlay playsInline className="size-full object-contain" />
  );
}
