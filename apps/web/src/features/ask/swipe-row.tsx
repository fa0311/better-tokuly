import { type PointerEvent, type ReactNode, useRef, useState } from "react";
import { cn } from "#/lib/cn";

export interface SwipeAction {
  label: string;
  icon: ReactNode;
  /** Background of the strip the swipe uncovers. */
  tint: string;
  onCommit(): void;
}

/** How far a row has to travel before letting go counts. */
const COMMIT_DISTANCE = 88;
const MAX_DISTANCE = 140;

/**
 * A list row that can be swiped sideways with a finger: dragging it uncovers
 * an action on either side, and letting go past the threshold runs it. Mouse
 * users are not offered the gesture (dragging selects text there); every
 * action must also be reachable from a button or menu inside the row.
 */
export function SwipeRow({ toLeft, toRight, children, className }: { /** Uncovered on the right when the row is pushed left. */ toLeft?: SwipeAction; /** Uncovered on the left when the row is pushed right. */ toRight?: SwipeAction; children: ReactNode; className?: string }) {
  const [offset, setOffset] = useState(0);
  const [dragging, setDragging] = useState(false);
  const start = useRef<{ x: number; y: number; id: number; locked: boolean } | null>(null);

  const clamp = (value: number) => Math.max(toLeft ? -MAX_DISTANCE : 0, Math.min(toRight ? MAX_DISTANCE : 0, value));

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.pointerType === "mouse" || (!toLeft && !toRight)) return;
    start.current = { x: event.clientX, y: event.clientY, id: event.pointerId, locked: false };
  };
  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const origin = start.current;
    if (!origin || origin.id !== event.pointerId) return;
    const dx = event.clientX - origin.x;
    const dy = event.clientY - origin.y;
    if (!origin.locked) {
      // Wait until the finger says which way it is going; a vertical move is a scroll.
      if (Math.abs(dx) < 10 && Math.abs(dy) < 10) return;
      if (Math.abs(dy) > Math.abs(dx)) {
        start.current = null;
        return;
      }
      origin.locked = true;
      setDragging(true);
      event.currentTarget.setPointerCapture?.(event.pointerId);
    }
    setOffset(clamp(dx));
  };
  const finish = (commit: boolean) => {
    if (!start.current) return;
    start.current = null;
    setDragging(false);
    const action = offset <= -COMMIT_DISTANCE ? toLeft : offset >= COMMIT_DISTANCE ? toRight : undefined;
    setOffset(0);
    if (commit) action?.onCommit();
  };

  const shown = offset < 0 ? toLeft : offset > 0 ? toRight : undefined;
  const armed = Math.abs(offset) >= COMMIT_DISTANCE;

  return (
    <div className={cn("relative overflow-hidden rounded-card", className)}>
      {shown && (
        <div aria-hidden className={cn("absolute inset-0 flex items-center px-6 font-black text-[#17122b]", shown.tint, offset < 0 ? "justify-end" : "justify-start")}>
          <span className={cn("flex items-center gap-2 transition-transform", armed && "scale-110")}>
            {shown.icon}
            {shown.label}
          </span>
        </div>
      )}
      <div
        data-testid="swipe-row"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={() => finish(true)}
        onPointerCancel={() => finish(false)}
        // Only real drags suppress the click that follows a release.
        onClickCapture={(event) => {
          if (dragging) event.stopPropagation();
        }}
        className={cn("relative touch-pan-y", !dragging && "transition-transform duration-200 ease-out-soft")}
        style={{ transform: offset ? `translateX(${offset}px)` : undefined }}
      >
        {children}
      </div>
    </div>
  );
}
