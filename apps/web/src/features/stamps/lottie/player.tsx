import { useEffect, useRef, useState } from "react";
import { frameAt } from "./animation";
import { loadDotLottie } from "./dotlottie";
import { drawFrame, type LoadedAnimation } from "./render";

/**
 * A dotLottie animation on a canvas that fills its box. It loops while
 * `playing`; otherwise it rests on a frame from the middle of the animation
 * (the start of a sticker animation is often empty).
 */
export default function LottiePlayer({ url, playing, label, className, onError }: { url: string; playing: boolean; label: string; className?: string; onError?(): void }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [loaded, setLoaded] = useState<LoadedAnimation | null>(null);

  useEffect(() => {
    let cancelled = false;
    loadDotLottie(url).then(
      (animation) => !cancelled && setLoaded(animation),
      () => !cancelled && onError?.(),
    );
    return () => {
      cancelled = true;
    };
  }, [url, onError]);

  useEffect(() => {
    const canvas = canvasRef.current;
    const context = canvas?.getContext("2d");
    if (!canvas || !context || !loaded) return;
    const { animation } = loaded;
    const still = animation.ip + (animation.op - animation.ip) / 2;
    const started = performance.now();
    let request = 0;
    let drawn = Number.NaN;
    let resized = true;

    const tick = (now: number) => {
      if (resized) {
        // Sharp on dense screens, but never more pixels than a stamp is worth.
        const ratio = Math.min(window.devicePixelRatio || 1, 2);
        const box = canvas.getBoundingClientRect();
        canvas.width = Math.max(1, Math.round(box.width * ratio));
        canvas.height = Math.max(1, Math.round(box.height * ratio));
        resized = false;
        drawn = Number.NaN;
      }
      // The animation's own frame rate is all it needs.
      const frame = playing ? Math.floor(frameAt(animation, now - started)) : still;
      if (frame !== drawn) {
        drawFrame(context, loaded, frame, canvas.width, canvas.height);
        drawn = frame;
      }
      if (playing) request = requestAnimationFrame(tick);
    };
    const observer = new ResizeObserver(() => {
      resized = true;
      if (!playing) request = requestAnimationFrame(tick);
    });
    observer.observe(canvas);
    request = requestAnimationFrame(tick);
    return () => {
      observer.disconnect();
      cancelAnimationFrame(request);
    };
  }, [loaded, playing]);

  return <canvas ref={canvasRef} role="img" aria-label={label} data-ready={loaded ? "" : undefined} className={className} />;
}
