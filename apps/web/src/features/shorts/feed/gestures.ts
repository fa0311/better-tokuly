import { type PointerEvent, useEffect, useRef } from "react";

/** How long a press lasts before it is a hold, and how long a tap waits for a second one. */
const HOLD_MS = 420;
const DOUBLE_TAP_MS = 260;
/** Movement (CSS pixels) after which a press is the start of a swipe, not a tap. */
const SLOP = 12;

export interface SurfaceGestures {
  /** One tap, once it is clear no second one follows. */
  onTap(): void;
  /** Two taps in a row; the position is relative to the surface. */
  onDoubleTap(x: number, y: number): void;
  onHoldStart(): void;
  onHoldEnd(): void;
}

/**
 * Tap, double tap and press-and-hold on the video, for touch and mouse
 * alike. A press that moves is a swipe and belongs to the scroller: it is
 * none of the three.
 */
export function useSurfaceGestures(handlers: SurfaceGestures) {
  const latest = useRef(handlers);
  useEffect(() => {
    latest.current = handlers;
  });
  const press = useRef<{ x: number; y: number; hold?: ReturnType<typeof setTimeout> } | null>(null);
  const holding = useRef(false);
  const lastTap = useRef<{ at: number; timer?: ReturnType<typeof setTimeout> }>({ at: 0 });

  const release = () => {
    clearTimeout(press.current?.hold);
    press.current = null;
    if (holding.current) {
      holding.current = false;
      latest.current.onHoldEnd();
    }
  };
  useEffect(
    () => () => {
      clearTimeout(press.current?.hold);
      clearTimeout(lastTap.current.timer);
    },
    [],
  );

  return {
    onPointerDown: (event: PointerEvent<HTMLElement>) => {
      if (event.pointerType === "mouse" && event.button !== 0) return;
      clearTimeout(press.current?.hold);
      press.current = {
        x: event.clientX,
        y: event.clientY,
        hold: setTimeout(() => {
          holding.current = true;
          latest.current.onHoldStart();
        }, HOLD_MS),
      };
    },
    onPointerMove: (event: PointerEvent<HTMLElement>) => {
      const start = press.current;
      if (!start || holding.current) return;
      if (Math.hypot(event.clientX - start.x, event.clientY - start.y) > SLOP) release();
    },
    onPointerUp: (event: PointerEvent<HTMLElement>) => {
      const start = press.current;
      const wasHolding = holding.current;
      release();
      if (!start || wasHolding) return;
      const now = event.timeStamp;
      if (now - lastTap.current.at < DOUBLE_TAP_MS) {
        clearTimeout(lastTap.current.timer);
        lastTap.current = { at: 0 };
        const box = event.currentTarget.getBoundingClientRect();
        latest.current.onDoubleTap(event.clientX - box.left, event.clientY - box.top);
        return;
      }
      lastTap.current = {
        at: now,
        timer: setTimeout(() => latest.current.onTap(), DOUBLE_TAP_MS),
      };
    },
    onPointerCancel: release,
    onPointerLeave: release,
  };
}
