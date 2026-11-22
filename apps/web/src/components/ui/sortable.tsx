import {
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
  type PointerEvent as ReactPointerEvent,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { cn } from "#/lib/cn";

/** Spread these on the element people grab to move an item (a grip button, or the item itself). */
export interface SortableGrip {
  onPointerDown(event: ReactPointerEvent<HTMLElement>): void;
  /** Arrow keys move the item one place. */
  onKeyDown(event: KeyboardEvent<HTMLElement>): void;
  style: CSSProperties;
}

export interface SortableItemState {
  grip: SortableGrip;
  /** This item is the one being dragged. */
  dragging: boolean;
}

export interface SortableProps<T> {
  items: T[];
  getKey(item: T): string;
  /** The new order, once an item was dropped somewhere else (or moved with the keyboard). */
  onReorder(items: T[]): void;
  /** Renders one item; put `state.grip` on what should be grabbed. */
  children(item: T, state: SortableItemState): ReactNode;
  /**
   * How a drag starts. `"handle"` (default): at once, for a dedicated grip —
   * the grip does not scroll the page under a finger. `"press"`: the whole
   * item is the grip; a finger has to rest on it for a moment first, so
   * swiping still scrolls and tapping still clicks.
   */
  grab?: "handle" | "press";
  disabled?: boolean;
  /** Layout of the container: a flex column, a grid… Items are its direct children. */
  className?: string;
  itemClassName?: string;
}

/** Pointer travel before a press becomes a drag (or, on touch, a scroll). */
const SLOP = 5;
const LONG_PRESS_MS = 260;
/** Distance from the scroll container's edge at which it starts scrolling during a drag. */
const SCROLL_EDGE = 72;

interface Drag {
  key: string;
  /** Where inside the item it was grabbed. */
  grabX: number;
  grabY: number;
  startX: number;
  startY: number;
  x: number;
  y: number;
  active: boolean;
  /** The order on screen while dragging. */
  order: string[];
  timer: ReturnType<typeof setTimeout> | undefined;
  frame: number;
  scroller: Element | null;
  stop(): void;
}

function scrollParent(element: Element): Element | null {
  for (let node = element.parentElement; node; node = node.parentElement) {
    const overflow = getComputedStyle(node).overflowY;
    if ((overflow === "auto" || overflow === "scroll") && node.scrollHeight > node.clientHeight)
      return node;
  }
  return null;
}

/**
 * A list or grid whose items can be dragged into a new order, with the
 * mouse, a finger or the arrow keys. The dragged item follows the pointer
 * and the others make room as it passes over them.
 */
export function Sortable<T>({
  items,
  getKey,
  onReorder,
  children,
  grab = "handle",
  disabled,
  className,
  itemClassName,
}: SortableProps<T>) {
  const elements = useRef(new Map<string, HTMLElement>());
  const drag = useRef<Drag | null>(null);
  /** The order shown while dragging; `null` when nothing is being dragged. */
  const [order, setOrder] = useState<string[] | null>(null);
  const [draggingKey, setDraggingKey] = useState<string | null>(null);

  const byKey = new Map(items.map((item) => [getKey(item), item]));
  const keys = (order ?? items.map(getKey)).filter((key) => byKey.has(key));

  /** Moves the dragged element under the pointer, whatever slot it currently occupies. */
  const follow = () => {
    const current = drag.current;
    const element = current && elements.current.get(current.key);
    if (!current?.active || !element) return;
    element.style.transform = "";
    const slot = element.getBoundingClientRect();
    element.style.transform = `translate(${current.x - current.grabX - slot.left}px, ${current.y - current.grabY - slot.top}px) scale(1.04)`;
  };

  // After the others made room, the dragged item sits in a new slot: re-aim before the browser paints.
  useLayoutEffect(follow);

  const finish = (commit: boolean) => {
    const current = drag.current;
    if (!current) return;
    current.stop();
    drag.current = null;
    const element = elements.current.get(current.key);
    if (element) element.style.transform = "";
    const dropped = current.order;
    setOrder(null);
    setDraggingKey(null);
    if (!current.active) return;
    // The click that ends a drag is not a click on the item.
    const swallow = (event: MouseEvent) => {
      event.stopPropagation();
      event.preventDefault();
    };
    window.addEventListener("click", swallow, { capture: true, once: true });
    setTimeout(() => window.removeEventListener("click", swallow, { capture: true }), 0);
    if (commit && dropped.some((key, index) => key !== getKey(items[index]!))) {
      onReorder(dropped.flatMap((key) => (byKey.has(key) ? [byKey.get(key)!] : [])));
    }
  };

  useEffect(() => () => drag.current?.stop(), []);

  const start = (key: string, event: ReactPointerEvent<HTMLElement>) => {
    const element = elements.current.get(key);
    if (
      disabled ||
      !element ||
      drag.current ||
      (event.pointerType === "mouse" && event.button !== 0)
    )
      return;
    const slot = element.getBoundingClientRect();
    const waitsForPress = grab === "press" && event.pointerType === "touch";

    const activate = () => {
      const current = drag.current;
      if (!current || current.active) return;
      current.active = true;
      current.scroller = scrollParent(element);
      setOrder(current.order);
      setDraggingKey(key);
      navigator.vibrate?.(8);
      const step = () => {
        const live = drag.current;
        if (!live) return;
        // Near the top or bottom edge, the page scrolls along.
        if (live.scroller) {
          const bounds = live.scroller.getBoundingClientRect();
          if (live.y < bounds.top + SCROLL_EDGE)
            live.scroller.scrollTop -= Math.ceil((bounds.top + SCROLL_EDGE - live.y) / 6);
          else if (live.y > bounds.bottom - SCROLL_EDGE)
            live.scroller.scrollTop += Math.ceil((live.y - (bounds.bottom - SCROLL_EDGE)) / 6);
        }
        follow();
        // Passing over another item takes its place.
        for (const [other, node] of elements.current) {
          if (other === live.key) continue;
          const box = node.getBoundingClientRect();
          if (live.x < box.left || live.x > box.right || live.y < box.top || live.y > box.bottom)
            continue;
          const target = live.order.indexOf(other);
          live.order = live.order.filter((item) => item !== live.key);
          live.order.splice(target, 0, live.key);
          setOrder(live.order);
          break;
        }
        live.frame = requestAnimationFrame(step);
      };
      current.frame = requestAnimationFrame(step);
    };

    const onMove = (moved: PointerEvent) => {
      const current = drag.current;
      if (!current) return;
      current.x = moved.clientX;
      current.y = moved.clientY;
      if (
        current.active ||
        Math.hypot(current.x - current.startX, current.y - current.startY) < SLOP
      )
        return;
      // A finger that moves before the press is up wants to scroll, not drag.
      if (waitsForPress) finish(false);
      else activate();
    };
    const onUp = () => finish(true);
    const onCancel = () => finish(false);
    const onKey = (pressed: globalThis.KeyboardEvent) => {
      if (pressed.key === "Escape") finish(false);
    };
    // Once dragging, the finger must not scroll the page as well.
    const onTouchMove = (touch: TouchEvent) => {
      if (drag.current?.active && touch.cancelable) touch.preventDefault();
    };

    drag.current = {
      key,
      grabX: event.clientX - slot.left,
      grabY: event.clientY - slot.top,
      startX: event.clientX,
      startY: event.clientY,
      x: event.clientX,
      y: event.clientY,
      active: false,
      order: items.map(getKey),
      timer: waitsForPress ? setTimeout(activate, LONG_PRESS_MS) : undefined,
      frame: 0,
      scroller: null,
      stop() {
        clearTimeout(this.timer);
        cancelAnimationFrame(this.frame);
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
        window.removeEventListener("pointercancel", onCancel);
        window.removeEventListener("keydown", onKey);
        window.removeEventListener("touchmove", onTouchMove);
      },
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
    window.addEventListener("keydown", onKey);
    window.addEventListener("touchmove", onTouchMove, { passive: false });
  };

  const nudge = (key: string, event: KeyboardEvent<HTMLElement>) => {
    const delta =
      event.key === "ArrowUp" || event.key === "ArrowLeft"
        ? -1
        : event.key === "ArrowDown" || event.key === "ArrowRight"
          ? 1
          : 0;
    if (delta === 0 || disabled) return;
    const index = keys.indexOf(key);
    const target = index + delta;
    if (index < 0 || target < 0 || target >= keys.length) return;
    event.preventDefault();
    const next = [...keys];
    next.splice(index, 1);
    next.splice(target, 0, key);
    onReorder(next.map((item) => byKey.get(item)!));
    // Keep the grip focused in its new place, so the keys can be pressed again.
    const focused = event.currentTarget;
    requestAnimationFrame(() => focused.focus());
  };

  return (
    <div className={className}>
      {keys.map((key) => {
        const dragging = key === draggingKey;
        return (
          <div
            key={key}
            ref={(node) => {
              if (node) elements.current.set(key, node);
              else elements.current.delete(key);
            }}
            data-dragging={dragging || undefined}
            className={cn(
              "relative",
              dragging && "z-10 drop-shadow-[0_10px_14px_rgb(23_18_43/0.3)]",
              itemClassName,
            )}
            // On touch, a press that turns into a drag must not also open the browser's context menu.
            onContextMenu={
              grab === "press" ? (event) => drag.current && event.preventDefault() : undefined
            }
          >
            {children(byKey.get(key)!, {
              dragging,
              grip: {
                onPointerDown: (event) => start(key, event),
                onKeyDown: (event) => nudge(key, event),
                style: {
                  touchAction: grab === "handle" ? "none" : "pan-y",
                  cursor: disabled ? undefined : dragging ? "grabbing" : "grab",
                  WebkitTouchCallout: "none",
                  userSelect: "none",
                },
              },
            })}
          </div>
        );
      })}
    </div>
  );
}
