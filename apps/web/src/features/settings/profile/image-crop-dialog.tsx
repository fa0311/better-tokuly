import { type PointerEvent as ReactPointerEvent, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { Slider } from "#/components/ui/slider";
import { Spinner } from "#/components/ui/spinner";
import { cn } from "#/lib/cn";
import { type CropFrame, type CropView, clampView, renderCrop, visibleRegion } from "./crop";

const MAX_ZOOM = 4;

/**
 * Lets the person choose which part of a picked image is used: drag to move,
 * pinch / scroll / slide to zoom. What is inside the frame when they confirm
 * is rendered to a JPEG in the browser and handed to `onCropped`.
 */
export function ImageCropDialog({
  file,
  frame,
  title,
  busy,
  onCancel,
  onCropped,
  onUnreadable,
}: {
  file: File;
  frame: CropFrame;
  title: string;
  /** The cropped image is being uploaded. */
  busy: boolean;
  onCancel(): void;
  onCropped(image: Blob): void;
  /** The file is not an image the browser can show. */
  onUnreadable(): void;
}) {
  const { t } = useTranslation(["settings", "common"]);
  const [image, setImage] = useState<HTMLImageElement | null>(null);
  const [view, setView] = useState<CropView>({ centerX: 0.5, centerY: 0.5, zoom: 1 });
  const [frameSize, setFrameSize] = useState({ width: 0, height: 0 });
  const frameRef = useRef<HTMLDivElement>(null);
  const pointers = useRef(new Map<number, { x: number; y: number }>());

  // Decode the picked file.
  useEffect(() => {
    const url = URL.createObjectURL(file);
    const element = new Image();
    element.onload = () => setImage(element);
    element.onerror = onUnreadable;
    element.src = url;
    return () => {
      // A load that is abandoned (the dialog closed, or the effect re-ran) must not report anything.
      element.onload = null;
      element.onerror = null;
      URL.revokeObjectURL(url);
    };
    // `onUnreadable` is only called from the load handlers of this file.
  }, [file]);

  // The frame is laid out by CSS; the maths needs its size in pixels.
  useEffect(() => {
    const element = frameRef.current;
    if (!element) return;
    const measure = () =>
      setFrameSize({ width: element.clientWidth, height: element.clientHeight });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [image]);

  const ready = image !== null && frameSize.width > 0;
  const source = image
    ? { width: image.naturalWidth, height: image.naturalHeight }
    : { width: 1, height: 1 };
  const update = (change: (current: CropView) => CropView) =>
    setView((current) => clampView(change(current), source, frame.aspect, MAX_ZOOM));

  // Scrolling over the frame zooms. A native listener, because React's wheel
  // handler is passive and could not stop the dialog from scrolling.
  useEffect(() => {
    const element = frameRef.current;
    if (!element) return;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      setView((current) =>
        clampView(
          { ...current, zoom: current.zoom * Math.exp(-event.deltaY * 0.0015) },
          source,
          frame.aspect,
          MAX_ZOOM,
        ),
      );
    };
    element.addEventListener("wheel", onWheel, { passive: false });
    return () => element.removeEventListener("wheel", onWheel);
  }, [source.width, source.height, frame.aspect]);

  // Frame pixels per image pixel right now.
  const region = visibleRegion(view, source, frame.aspect);
  const scale = frameSize.width / region.width;

  const onPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    event.currentTarget.setPointerCapture(event.pointerId);
    pointers.current.set(event.pointerId, { x: event.clientX, y: event.clientY });
  };
  const onPointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const previous = pointers.current.get(event.pointerId);
    if (!previous) return;
    const position = { x: event.clientX, y: event.clientY };
    const others = [...pointers.current.entries()]
      .filter(([id]) => id !== event.pointerId)
      .map(([, point]) => point);
    pointers.current.set(event.pointerId, position);
    const other = others[0];
    if (other) {
      // Two fingers: the change of their distance is the zoom.
      const before = Math.hypot(previous.x - other.x, previous.y - other.y);
      const after = Math.hypot(position.x - other.x, position.y - other.y);
      if (before > 0) update((current) => ({ ...current, zoom: current.zoom * (after / before) }));
      return;
    }
    const dx = (position.x - previous.x) / scale / source.width;
    const dy = (position.y - previous.y) / scale / source.height;
    update((current) => ({
      ...current,
      centerX: current.centerX - dx,
      centerY: current.centerY - dy,
    }));
  };
  const onPointerEnd = (event: ReactPointerEvent<HTMLDivElement>) => {
    pointers.current.delete(event.pointerId);
  };
  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const step = 0.03 / view.zoom;
    const moves: Record<string, [number, number]> = {
      ArrowLeft: [-step, 0],
      ArrowRight: [step, 0],
      ArrowUp: [0, -step],
      ArrowDown: [0, step],
    };
    const move = moves[event.key];
    if (!move) return;
    event.preventDefault();
    update((current) => ({
      ...current,
      centerX: current.centerX + move[0],
      centerY: current.centerY + move[1],
    }));
  };

  const confirm = async () => {
    if (!image) return;
    onCropped(await renderCrop(image, view, frame));
  };

  return (
    <Dialog
      open
      onOpenChange={(open) => !open && !busy && onCancel()}
      title={title}
      description={t("settings:profile.crop.lead")}
      footer={
        <>
          <Button variant="ghost" onClick={onCancel} disabled={busy}>
            {t("common:actions.cancel")}
          </Button>
          <Button onClick={confirm} loading={busy} disabled={!ready}>
            {busy ? t("settings:profile.crop.uploading") : t("settings:profile.crop.confirm")}
          </Button>
        </>
      }
    >
      <div
        ref={frameRef}
        role="application"
        aria-label={t("settings:profile.crop.frame")}
        // biome-ignore lint/a11y/noNoninteractiveTabindex: the frame is moved with the arrow keys.
        tabIndex={0}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerEnd}
        onPointerCancel={onPointerEnd}
        onKeyDown={onKeyDown}
        className={cn(
          "sticker relative mx-auto w-full cursor-grab touch-none select-none overflow-hidden bg-surface-3 active:cursor-grabbing",
          frame.shape === "avatar" ? "max-w-64 rounded-[27%]" : "rounded-2xl",
        )}
        style={{ aspectRatio: String(frame.aspect) }}
      >
        {ready ? (
          <img
            src={image.src}
            alt=""
            draggable={false}
            className="pointer-events-none absolute top-0 left-0 max-w-none origin-top-left"
            style={{
              width: source.width * scale,
              height: source.height * scale,
              transform: `translate(${-region.x * scale}px, ${-region.y * scale}px)`,
            }}
          />
        ) : (
          <div className="absolute inset-0 grid place-items-center text-ink-faint">
            <Spinner className="size-7" />
          </div>
        )}
      </div>
      <div className="mt-4 flex items-center gap-3">
        <span aria-hidden className="text-sm">
          🔍
        </span>
        <Slider
          label={t("settings:profile.crop.zoom")}
          value={view.zoom}
          min={1}
          max={MAX_ZOOM}
          step={0.01}
          valueText={`${Math.round(view.zoom * 100)}%`}
          disabled={!ready || busy}
          onChange={(zoom) => update((current) => ({ ...current, zoom }))}
        />
      </div>
    </Dialog>
  );
}
