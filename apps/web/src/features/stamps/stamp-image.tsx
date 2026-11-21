import { common } from "@tokuly/api";
import { ImageOff } from "lucide-react";
import { type CSSProperties, lazy, type RefObject, Suspense, useEffect, useRef, useState } from "react";
import { cn } from "#/lib/cn";

const LottiePlayer = lazy(() => import("./lottie/player"));

export interface StampImageProps {
  /** The stamp to show: the `StampRef` every domain embeds (messages, comments, notes, reactions). */
  stamp: common.StampRef;
  /**
   * Edge of the square box in CSS pixels. Without it the stamp fills the
   * width of its parent.
   */
  size?: number;
  /**
   * When animated and Lottie stamps move: `"visible"` (default) while they
   * are on screen and the person has not turned motion off, `"always"`
   * (previews the person opened on purpose), or `"never"` (a still).
   */
  play?: "visible" | "always" | "never";
  /**
   * The stamp's processed image, when the caller has the full `stamp.Stamp`:
   * lets small tiles load a small rendition instead of the full picture
   * (without `size`, a grid tile of about 80 px is assumed).
   */
  image?: common.Image;
  className?: string;
}

/**
 * Renders a stamp in its format: a still image, an animated image, or a
 * Lottie animation. The stamp is contained in a square box and keeps its
 * proportions; its name is the alternative text.
 */
export function StampImage({ stamp, size, play = "visible", image, className }: StampImageProps) {
  const box = useRef<HTMLSpanElement>(null);
  const animated = stamp.format === common.StampFormat.ANIMATED || stamp.format === common.StampFormat.LOTTIE;
  const onScreen = useOnScreen(box, animated);
  const [failed, setFailed] = useState(false);
  // Lottie files are fetched only once the stamp has come near the screen.
  const [seen, setSeen] = useState(false);
  if (onScreen && !seen) setSeen(true);

  const moving = animated && (play === "always" || (play === "visible" && onScreen && !motionOff()));
  const style: CSSProperties | undefined = size ? { width: size, height: size } : undefined;
  const boxClass = cn("relative inline-block shrink-0 select-none align-middle", !size && "aspect-square w-full", className);

  if (failed || (!stamp.imageUrl && !stamp.animationUrl)) {
    return (
      <span ref={box} role="img" aria-label={stamp.name} style={style} className={cn(boxClass, "grid place-items-center rounded-2xl border-2 border-dashed border-line text-ink-faint")}>
        <ImageOff className="size-1/3" />
      </span>
    );
  }

  if (stamp.format === common.StampFormat.LOTTIE) {
    return (
      <span ref={box} style={style} className={boxClass}>
        {seen || play === "always" ? (
          <Suspense fallback={null}>
            <LottiePlayer url={stamp.animationUrl} playing={moving} label={stamp.name} className="absolute inset-0 size-full" onError={() => setFailed(true)} />
          </Suspense>
        ) : (
          stamp.imageUrl && <img src={stamp.imageUrl} alt={stamp.name} className="absolute inset-0 size-full object-contain" draggable={false} />
        )}
      </span>
    );
  }

  const still = stillUrl(stamp, image, size);
  return (
    <span ref={box} style={style} className={boxClass}>
      <img
        src={moving && stamp.animationUrl ? stamp.animationUrl : still}
        alt={stamp.name}
        className="absolute inset-0 size-full object-contain"
        loading="lazy"
        decoding="async"
        draggable={false}
        onError={() => setFailed(true)}
      />
    </span>
  );
}

/** The size assumed for a stamp that fills its parent, in CSS pixels. */
const GRID_TILE = 80;

/** The smallest rendition that still looks sharp at `size` CSS pixels. */
function stillUrl(stamp: common.StampRef, image: common.Image | undefined, size: number | undefined): string {
  const fallback = stamp.imageUrl || stamp.animationUrl;
  if (!image) return fallback;
  const wanted = (size ?? GRID_TILE) * Math.min(window.devicePixelRatio || 1, 2);
  const fitting = image.variants.filter((variant) => variant.size >= wanted).sort((a, b) => a.size - b.size)[0];
  return fitting?.url || fallback;
}

/** The person asked for no motion: ドパガキ度 0, or the system setting. */
function motionOff(): boolean {
  return document.documentElement.dataset.dopa === "0" || window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

// One observer for every stamp on the page.
const listeners = new WeakMap<Element, (visible: boolean) => void>();
let observer: IntersectionObserver | null = null;

function watch(element: Element, listener: (visible: boolean) => void): () => void {
  observer ??= new IntersectionObserver(
    (entries) => {
      for (const entry of entries) listeners.get(entry.target)?.(entry.isIntersecting);
    },
    // Start a little before the stamp scrolls in, so it is already moving when it arrives.
    { rootMargin: "120px" },
  );
  listeners.set(element, listener);
  observer.observe(element);
  return () => {
    listeners.delete(element);
    observer?.unobserve(element);
  };
}

/** Whether the element is on (or near) the screen; only watched while `enabled`. */
function useOnScreen(ref: RefObject<Element | null>, enabled: boolean): boolean {
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    const element = ref.current;
    if (!enabled || !element) return;
    const stop = watch(element, setVisible);
    return () => {
      stop();
      setVisible(false);
    };
  }, [ref, enabled]);
  return visible;
}
