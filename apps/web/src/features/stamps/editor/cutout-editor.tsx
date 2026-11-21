import { Eraser, Hand, Maximize, Paintbrush, Redo2, RotateCcw, Sparkles, Undo2, Wand2, ZoomIn, ZoomOut } from "lucide-react";
import { type ComponentType, type PointerEvent, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Slider } from "#/components/ui/slider";
import { PaneSpinner } from "#/components/ui/spinner";
import { SwitchRow } from "#/components/ui/switch";
import { cn } from "#/lib/cn";
import {
  addOutline,
  applyMask,
  contentBounds,
  cornerSeeds,
  fitPlacement,
  fitWithin,
  floodErase,
  fullMask,
  History,
  paintMask,
  type Placement,
  type Point,
  type Rect,
  STAMP_SIZE,
  toPicture,
  zoomPlacement,
} from "./image-ops";

type Tool = "move" | "erase" | "restore" | "wand";

/** Longest edge of the picture while it is being edited; larger pictures are scaled down first. */
const WORK_LIMIT = 1024;
/** Thickness of the white sticker outline, in pixels of the exported stamp. */
const OUTLINE = 10;
/** Room left around the subject when it is fitted to the frame (the outline needs some). */
const FIT_MARGIN = 30;
const HISTORY_STEPS = 20;

/** Everything about the picture being edited that is not React state. */
interface Session {
  /** The picture, scaled to the working size. Never changes. */
  source: ImageData;
  /** What is kept (255) and cut away (0). */
  mask: Uint8Array;
  history: History<Uint8Array>;
  /** The mask before the last magic removal, while its tolerance can still be adjusted. */
  magicBase: Uint8Array | null;
  /** The cut-out picture at working size. */
  cut: HTMLCanvasElement;
  cutContext: CanvasRenderingContext2D;
  cutPixels: ImageData;
  /** Where the picture sits in the square frame. */
  placement: Placement;
  /** Scale at which the whole picture fits the frame: the reference for zoom limits. */
  baseScale: number;
  outline: boolean;
  frame: number;
}

interface Gesture {
  kind: "pan" | "stroke" | "tap" | "pinch" | "idle";
  last: Point;
  /** Fingers apart, for a pinch. */
  spread: number;
  /** The mask before the stroke, to take it back if the stroke turns into a pinch. */
  before: Uint8Array | null;
}

async function decode(file: Blob): Promise<ImageBitmap | HTMLImageElement> {
  try {
    return await createImageBitmap(file, { imageOrientation: "from-image" });
  } catch {
    // Formats the bitmap decoder refuses (some SVGs) may still load as an <img>.
    const image = new Image();
    const url = URL.createObjectURL(file);
    try {
      image.src = url;
      await image.decode();
      return image;
    } finally {
      URL.revokeObjectURL(url);
    }
  }
}

/**
 * The cut-out editor: frames a picture in the square stamp (move, pinch or
 * scroll to zoom), removes its background by brush or by magic, optionally
 * adds a white sticker outline, and hands back a transparent 512 px image.
 */
export function CutoutEditor({ file, onBack, onDone }: { file: Blob; onBack(): void; onDone(image: Blob): void }) {
  const { t } = useTranslation("stamps");
  const stageRef = useRef<HTMLCanvasElement>(null);
  const session = useRef<Session | null>(null);
  const pointers = useRef(new Map<number, Point>());
  const gesture = useRef<Gesture | null>(null);

  const [status, setStatus] = useState<"loading" | "ready" | "failed">("loading");
  const [tool, setTool] = useState<Tool>("move");
  const [brush, setBrush] = useState(44);
  const [tolerance, setTolerance] = useState(36);
  const [outline, setOutline] = useState(true);
  const [steps, setSteps] = useState({ undo: false, redo: false });
  const [cursor, setCursor] = useState<Point | null>(null);
  const [exporting, setExporting] = useState(false);

  // ── Drawing ──

  const draw = () => {
    const current = session.current;
    const context = stageRef.current?.getContext("2d", { willReadFrequently: true });
    if (!current || !context) return;
    const { placement } = current;
    context.setTransform(1, 0, 0, 1, 0, 0);
    context.clearRect(0, 0, STAMP_SIZE, STAMP_SIZE);
    context.imageSmoothingQuality = "high";
    context.setTransform(placement.scale, 0, 0, placement.scale, placement.x, placement.y);
    context.drawImage(current.cut, 0, 0);
    context.setTransform(1, 0, 0, 1, 0, 0);
    if (current.outline) {
      const pixels = context.getImageData(0, 0, STAMP_SIZE, STAMP_SIZE);
      addOutline(pixels, OUTLINE);
      context.putImageData(pixels, 0, 0);
    }
  };

  const requestDraw = () => {
    const current = session.current;
    if (!current) return;
    cancelAnimationFrame(current.frame);
    current.frame = requestAnimationFrame(draw);
  };

  /** Brings the cut-out canvas in line with the mask, everywhere or in `area`. */
  const refreshCut = (area?: Rect) => {
    const current = session.current;
    if (!current) return;
    applyMask(current.source, current.mask, current.cutPixels.data, area);
    if (area) current.cutContext.putImageData(current.cutPixels, 0, 0, area.x, area.y, area.width, area.height);
    else current.cutContext.putImageData(current.cutPixels, 0, 0);
    requestDraw();
  };

  const syncSteps = () => {
    const history = session.current?.history;
    if (history) setSteps({ undo: history.canUndo, redo: history.canRedo });
  };

  /** Records the mask as a new undo step. */
  const commit = () => {
    const current = session.current;
    if (!current) return;
    current.history.push(current.mask.slice());
    current.magicBase = null;
    syncSteps();
  };

  const fit = () => {
    const current = session.current;
    if (!current) return;
    const whole = { x: 0, y: 0, width: current.source.width, height: current.source.height };
    current.placement = fitPlacement(contentBounds(current.source, current.mask) ?? whole, STAMP_SIZE, FIT_MARGIN);
    requestDraw();
  };

  // ── Loading the picture ──

  useEffect(() => {
    let cancelled = false;
    setStatus("loading");
    void (async () => {
      try {
        const picture = await decode(file);
        if (cancelled) return;
        const size = fitWithin(picture.width, picture.height, WORK_LIMIT);
        const cut = document.createElement("canvas");
        cut.width = size.width;
        cut.height = size.height;
        const cutContext = cut.getContext("2d", { willReadFrequently: true });
        if (!cutContext) throw new Error("no canvas");
        cutContext.imageSmoothingQuality = "high";
        cutContext.drawImage(picture, 0, 0, size.width, size.height);
        const source = cutContext.getImageData(0, 0, size.width, size.height);
        const mask = fullMask(size.width, size.height);
        const placement = fitPlacement({ x: 0, y: 0, width: size.width, height: size.height }, STAMP_SIZE, FIT_MARGIN);
        session.current = {
          source,
          mask,
          history: new History(mask.slice(), HISTORY_STEPS),
          magicBase: null,
          cut,
          cutContext,
          cutPixels: cutContext.createImageData(size.width, size.height),
          placement,
          baseScale: placement.scale,
          outline: true,
          frame: 0,
        };
        setStatus("ready");
      } catch {
        if (!cancelled) setStatus("failed");
      }
    })();
    return () => {
      cancelled = true;
      if (session.current) cancelAnimationFrame(session.current.frame);
      session.current = null;
    };
  }, [file]);

  // The stage exists only once the picture is ready: draw the first frame then, and after the outline toggles.
  useEffect(() => {
    if (status !== "ready" || !session.current) return;
    session.current.outline = outline;
    refreshCut();
  }, [status, outline]);

  // ── Actions ──

  const undo = (redo: boolean) => {
    const current = session.current;
    const state = redo ? current?.history.redo() : current?.history.undo();
    if (!current || !state) return;
    current.mask.set(state);
    current.magicBase = null;
    syncSteps();
    refreshCut();
  };

  const reset = () => {
    const current = session.current;
    if (!current) return;
    current.mask.fill(255);
    commit();
    refreshCut();
    fit();
  };

  /** Magic removal from the corners. With `adjust`, it redoes the last removal with a new tolerance. */
  const magic = (value: number, adjust: boolean) => {
    const current = session.current;
    if (!current) return;
    const base = adjust && current.magicBase ? current.magicBase : current.mask.slice();
    const next = base.slice();
    const erased = floodErase(current.source, next, cornerSeeds(current.source.width, current.source.height), value);
    if (!adjust && erased === 0) {
      toast(t("editor.magicNothing"));
      return;
    }
    current.mask.set(next);
    if (adjust && current.magicBase) {
      current.history.amend(next);
    } else {
      current.history.push(next);
      current.magicBase = base;
      syncSteps();
    }
    refreshCut();
    fit();
  };

  const zoom = (factor: number, around: Point = { x: STAMP_SIZE / 2, y: STAMP_SIZE / 2 }) => {
    const current = session.current;
    if (!current) return;
    current.placement = zoomPlacement(current.placement, factor, around, current.baseScale * 0.25, current.baseScale * 10);
    requestDraw();
  };

  const finish = async () => {
    const stage = stageRef.current;
    const context = stage?.getContext("2d", { willReadFrequently: true });
    if (!stage || !context || !session.current) return;
    cancelAnimationFrame(session.current.frame);
    draw();
    if (!contentBounds(context.getImageData(0, 0, STAMP_SIZE, STAMP_SIZE))) {
      toast.error(t("editor.nothingLeft"));
      return;
    }
    setExporting(true);
    // WebP keeps the transparency at a fraction of a PNG's size; browsers that cannot write it fall back to PNG.
    const image = await new Promise<Blob | null>((resolve) => stage.toBlob(resolve, "image/webp", 0.92));
    setExporting(false);
    if (image) onDone(image);
    else toast.error(t("editor.exportFailed"));
  };

  // Undo and redo from the keyboard.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== "z") return;
      event.preventDefault();
      undo(event.shiftKey);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // Scrolling over the stage zooms (a native listener: React's wheel events cannot stop the page from scrolling).
  useEffect(() => {
    const stage = stageRef.current;
    if (!stage || status !== "ready") return;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      const box = stage.getBoundingClientRect();
      zoom(Math.exp(-event.deltaY * 0.0015), { x: ((event.clientX - box.left) * STAMP_SIZE) / box.width, y: ((event.clientY - box.top) * STAMP_SIZE) / box.height });
    };
    stage.addEventListener("wheel", onWheel, { passive: false });
    return () => stage.removeEventListener("wheel", onWheel);
  });

  // ── Pointer input ──

  /** A pointer position in pixels of the frame. */
  const framePoint = (event: PointerEvent<HTMLCanvasElement>): Point => {
    const box = event.currentTarget.getBoundingClientRect();
    return { x: ((event.clientX - box.left) * STAMP_SIZE) / box.width, y: ((event.clientY - box.top) * STAMP_SIZE) / box.height };
  };

  const stroke = (from: Point, to: Point) => {
    const current = session.current;
    if (!current || (tool !== "erase" && tool !== "restore")) return;
    const { source, mask, placement } = current;
    // The brush size is in frame pixels, the mask in picture pixels.
    refreshCut(paintMask(mask, source.width, source.height, toPicture(placement, from), toPicture(placement, to), brush / 2 / placement.scale, tool));
  };

  const onPointerDown = (event: PointerEvent<HTMLCanvasElement>) => {
    const current = session.current;
    if (!current) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    const point = framePoint(event);
    pointers.current.set(event.pointerId, point);

    if (pointers.current.size === 2) {
      // A second finger: whatever the first one started becomes a pinch.
      const active = gesture.current;
      if (active?.kind === "stroke" && active.before) {
        current.mask.set(active.before);
        refreshCut();
      }
      const [a, b] = [...pointers.current.values()] as [Point, Point];
      gesture.current = { kind: "pinch", last: { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 }, spread: Math.hypot(a.x - b.x, a.y - b.y), before: null };
    } else if (pointers.current.size === 1) {
      if (tool === "move") gesture.current = { kind: "pan", last: point, spread: 0, before: null };
      else if (tool === "wand") gesture.current = { kind: "tap", last: point, spread: 0, before: null };
      else {
        gesture.current = { kind: "stroke", last: point, spread: 0, before: current.mask.slice() };
        stroke(point, point);
      }
    }
  };

  const onPointerMove = (event: PointerEvent<HTMLCanvasElement>) => {
    const current = session.current;
    const point = framePoint(event);
    if (event.pointerType !== "touch" || pointers.current.has(event.pointerId)) setCursor(point);
    const active = gesture.current;
    if (!current || !active || !pointers.current.has(event.pointerId)) return;
    pointers.current.set(event.pointerId, point);

    if (active.kind === "pinch" && pointers.current.size === 2) {
      const [a, b] = [...pointers.current.values()] as [Point, Point];
      const middle = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
      const spread = Math.hypot(a.x - b.x, a.y - b.y);
      current.placement = { ...current.placement, x: current.placement.x + middle.x - active.last.x, y: current.placement.y + middle.y - active.last.y };
      if (active.spread > 0) zoom(spread / active.spread, middle);
      active.last = middle;
      active.spread = spread;
      requestDraw();
    } else if (active.kind === "pan") {
      current.placement = { ...current.placement, x: current.placement.x + point.x - active.last.x, y: current.placement.y + point.y - active.last.y };
      active.last = point;
      requestDraw();
    } else if (active.kind === "stroke") {
      stroke(active.last, point);
      active.last = point;
    }
  };

  const onPointerUp = (event: PointerEvent<HTMLCanvasElement>) => {
    const current = session.current;
    const active = gesture.current;
    if (!pointers.current.delete(event.pointerId) || !current || !active) return;
    if (event.pointerType === "touch") setCursor(null);

    if (active.kind === "stroke") commit();
    if (active.kind === "tap" && event.type === "pointerup") {
      // Magic wand: clears the area of similar colour around the tapped spot.
      const erased = floodErase(current.source, current.mask, [toPicture(current.placement, framePoint(event))], tolerance);
      if (erased > 0) {
        commit();
        refreshCut();
      }
    }
    // After a pinch, the finger that is still down does nothing more.
    gesture.current = pointers.current.size === 0 ? null : { kind: "idle", last: active.last, spread: 0, before: null };
  };

  // ── UI ──

  if (status === "failed") {
    return (
      <div className="flex flex-col items-center gap-4 py-8 text-center">
        <p className="text-5xl">🙈</p>
        <p className="font-black">{t("editor.loadFailed")}</p>
        <Button variant="secondary" onClick={onBack}>
          {t("editor.pickAnother")}
        </Button>
      </div>
    );
  }

  const brushing = tool === "erase" || tool === "restore";
  return (
    <div className="flex flex-col gap-3">
      {/* As large as the sheet allows while every control below still fits without scrolling. */}
      <div className="relative mx-auto aspect-square w-full" style={{ maxWidth: "min(26rem, max(11rem, calc(92dvh - 26.5rem)))" }}>
        {/* The checkerboard shows what is transparent. */}
        <div
          className="sticker absolute inset-0 overflow-hidden rounded-[1.75rem] bg-surface"
          style={{
            backgroundImage: "repeating-conic-gradient(var(--surface-3) 0% 25%, transparent 0% 50%)",
            backgroundSize: "24px 24px",
          }}
        >
          {status === "loading" ? (
            <PaneSpinner />
          ) : (
            <canvas
              ref={stageRef}
              width={STAMP_SIZE}
              height={STAMP_SIZE}
              role="img"
              aria-label={t("editor.stage")}
              data-testid="cutout-stage"
              className={cn("size-full touch-none select-none", tool === "move" ? "cursor-grab active:cursor-grabbing" : brushing ? "cursor-none" : "cursor-crosshair")}
              onPointerDown={onPointerDown}
              onPointerMove={onPointerMove}
              onPointerUp={onPointerUp}
              onPointerCancel={onPointerUp}
              onPointerLeave={() => pointers.current.size === 0 && setCursor(null)}
            />
          )}
          {brushing && cursor && (
            <span
              aria-hidden
              className="pointer-events-none absolute -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-white shadow-[0_0_0_1.5px_rgb(23_18_43)]"
              style={{
                left: `${(cursor.x / STAMP_SIZE) * 100}%`,
                top: `${(cursor.y / STAMP_SIZE) * 100}%`,
                width: `${(brush / STAMP_SIZE) * 100}%`,
                height: `${(brush / STAMP_SIZE) * 100}%`,
              }}
            />
          )}
        </div>
      </div>

      <div className="flex items-center justify-between gap-2">
        <div role="radiogroup" aria-label={t("editor.tools")} className="inline-flex gap-1 rounded-full bg-surface-2 p-1">
          <ToolButton current={tool} value="move" icon={Hand} label={t("editor.tool.move")} onSelect={setTool} />
          <ToolButton current={tool} value="erase" icon={Eraser} label={t("editor.tool.erase")} onSelect={setTool} />
          <ToolButton current={tool} value="restore" icon={Paintbrush} label={t("editor.tool.restore")} onSelect={setTool} />
          <ToolButton current={tool} value="wand" icon={Wand2} label={t("editor.tool.wand")} onSelect={setTool} />
        </div>
        <div className="flex gap-1">
          <Button size="icon-sm" variant="ghost" aria-label={t("editor.undo")} disabled={!steps.undo} onClick={() => undo(false)}>
            <Undo2 className="size-5" />
          </Button>
          <Button size="icon-sm" variant="ghost" aria-label={t("editor.redo")} disabled={!steps.redo} onClick={() => undo(true)}>
            <Redo2 className="size-5" />
          </Button>
          <Button size="icon-sm" variant="ghost" aria-label={t("editor.reset")} title={t("editor.reset")} disabled={status !== "ready"} onClick={reset}>
            <RotateCcw className="size-5" />
          </Button>
        </div>
      </div>

      <div className="flex min-h-11 items-center gap-3 rounded-2xl bg-surface-2 px-3 py-1.5">
        {tool === "move" ? (
          <>
            <p className="min-w-0 flex-1 text-sm font-bold text-ink-soft">{t("editor.moveHint")}</p>
            <Button size="icon-sm" variant="ghost" aria-label={t("editor.zoomOut")} onClick={() => zoom(1 / 1.25)}>
              <ZoomOut className="size-5" />
            </Button>
            <Button size="icon-sm" variant="ghost" aria-label={t("editor.zoomIn")} onClick={() => zoom(1.25)}>
              <ZoomIn className="size-5" />
            </Button>
            <Button size="icon-sm" variant="ghost" aria-label={t("editor.fit")} onClick={fit}>
              <Maximize className="size-5" />
            </Button>
          </>
        ) : brushing ? (
          <LabelledSlider label={t("editor.brushSize")} min={8} max={140} value={brush} onChange={setBrush} />
        ) : (
          <p className="min-w-0 flex-1 text-sm font-bold text-ink-soft">{t("editor.wandHint")}</p>
        )}
      </div>

      <div className="flex items-center gap-3 rounded-2xl border-2 border-line py-0.5 pr-3 pl-2">
        <Button variant="lime" size="sm" className="shrink-0" icon={<Sparkles className="size-4" />} disabled={status !== "ready"} onClick={() => magic(tolerance, false)}>
          {t("editor.magic")}
        </Button>
        <LabelledSlider
          label={t("editor.tolerance")}
          min={4}
          max={120}
          value={tolerance}
          onChange={(value) => {
            setTolerance(value);
            // Still on the magic removal: the slider tunes it live.
            if (session.current?.magicBase) magic(value, true);
          }}
        />
      </div>
      <div className="rounded-2xl border-2 border-line">
        <SwitchRow label={t("editor.outline")} checked={outline} onChange={setOutline} />
      </div>

      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onBack} disabled={exporting}>
          {t("editor.pickAnother")}
        </Button>
        <Button onClick={finish} loading={exporting} disabled={status !== "ready"}>
          {t("common:actions.next")}
        </Button>
      </div>
    </div>
  );
}

function ToolButton({ current, value, icon: Icon, label, onSelect }: { current: Tool; value: Tool; icon: ComponentType<{ className?: string }>; label: string; onSelect(tool: Tool): void }) {
  const active = current === value;
  return (
    // biome-ignore lint/a11y/useSemanticElements: a segmented control of icon buttons, not a form field.
    <button
      type="button"
      role="radio"
      aria-checked={active}
      aria-label={label}
      title={label}
      onClick={() => onSelect(value)}
      className={cn("flex h-9 items-center gap-1.5 rounded-full px-3 text-sm font-bold transition-colors", active ? "sticker-sm bg-surface text-ink" : "text-ink-soft hover:text-ink")}
    >
      <Icon className="size-[18px]" />
      {active && <span className="max-sm:hidden">{label}</span>}
    </button>
  );
}

/** A slider with its name written beside it. */
function LabelledSlider({ label, min, max, value, onChange }: { label: string; min: number; max: number; value: number; onChange(value: number): void }) {
  return (
    <div className="flex min-w-0 flex-1 items-center gap-2.5 text-sm font-bold text-ink-soft">
      <span className="shrink-0">{label}</span>
      <Slider label={label} min={min} max={max} value={value} onChange={onChange} className="min-w-0 flex-1" />
    </div>
  );
}
