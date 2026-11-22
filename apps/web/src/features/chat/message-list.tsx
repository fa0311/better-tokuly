import { useVirtualizer } from "@tanstack/react-virtual";
import { chat } from "@tokuly/api";
import { type Ref, useEffect, useImperativeHandle, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Spinner } from "#/components/ui/spinner";
import { MessageItem } from "./message-item";
import type { Row } from "./timeline";
import { dayLabel } from "./text";

/** What the screen can ask of the list. */
export interface MessageListHandle {
  /** Goes to the newest message (after sending, or "jump to latest"). */
  scrollToEnd(smooth?: boolean): void;
}

/** How close to the end (px) still counts as "at the bottom". */
const END_THRESHOLD = 96;

/** A first guess at a row's height, so the scrollbar is roughly right before rows are measured. */
function estimate(row: Row | undefined): number {
  if (!row) return 48;
  if (row.type === "day") return 44;
  if (row.type === "unread") return 40;
  const { message } = row;
  switch (message.kind) {
    case chat.MessageKind.STAMP:
      return 150;
    case chat.MessageKind.MEDIA:
      return 230;
    case chat.MessageKind.LOCATION:
      return 170;
    case chat.MessageKind.POV_REQUEST:
      return 200;
    case chat.MessageKind.SYSTEM:
      return 40;
    case chat.MessageKind.CALL:
      return 64;
    default:
      return 46 + Math.floor(message.text.length / 28) * 22 + (message.replyTo ? 48 : 0) + (message.linkPreview ? 150 : 0);
  }
}

/**
 * The messages of a conversation, virtualised: only the rows near the
 * viewport exist. The list is anchored to its end, like every chat: it opens
 * at the bottom (or at the unread divider), stays there while messages
 * arrive, keeps its place when older messages are added on top, and tells
 * the screen when more should be loaded in either direction.
 */
export function MessageList({
  ref,
  rows,
  hasOlder,
  hasNewer,
  focusId,
  focusNonce,
  onNearStart,
  onNearEnd,
  onAtEndChange,
  onOpenLocation,
}: {
  ref?: Ref<MessageListHandle>;
  rows: Row[];
  hasOlder: boolean;
  hasNewer: boolean;
  /** The message to bring into view and mark; `focusNonce` changes with every request. */
  focusId?: string;
  focusNonce: number;
  onNearStart(): void;
  onNearEnd(): void;
  onAtEndChange(atEnd: boolean): void;
  onOpenLocation(location: chat.LocationShare): void;
}) {
  // The virtualizer is a mutable object whose answers change without its identity changing,
  // which the React Compiler cannot see: this component is rendered without memoization.
  "use no memo";
  const { t } = useTranslation("chat");
  const scroller = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  /** The reader is following the newest messages (see the effect below). */
  const pinned = useRef(false);
  /** Until when the list is still settling at its end after being sent there. */
  const settlingUntil = useRef(0);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scroller.current,
    estimateSize: (index) => estimate(rows[index]),
    getItemKey: (index) => rows[index]?.key ?? index,
    overscan: 6,
    paddingStart: 8,
    paddingEnd: 14,
    anchorTo: "end",
    followOnAppend: true,
    scrollEndThreshold: END_THRESHOLD,
  });
  const items = virtualizer.getVirtualItems();

  const toEnd = (smooth?: boolean) => {
    pinned.current = true;
    settlingUntil.current = smooth ? 0 : performance.now() + 1500;
    virtualizer.scrollToEnd({ behavior: smooth ? "smooth" : "auto" });
  };
  useImperativeHandle(ref, () => ({ scrollToEnd: toEnd }));

  // Where the list opens: at the message asked for, else at "unread from here", else at the newest message.
  const opened = useRef(false);
  const [ready, setReady] = useState(false);
  useLayoutEffect(() => {
    if (opened.current || rows.length === 0) return;
    opened.current = true;
    // Two frames later the rows are measured and the list is where it opens.
    requestAnimationFrame(() => requestAnimationFrame(() => setReady(true)));
    const focus = focusId ? rows.findIndex((row) => row.type === "message" && row.message.id === focusId) : -1;
    const unread = rows.findIndex((row) => row.type === "unread");
    if (focus >= 0) virtualizer.scrollToIndex(focus, { align: "center" });
    else if (unread >= 0) virtualizer.scrollToIndex(unread, { align: "start" });
    else toEnd();
  });

  // A later request to show a message (a reply's original, a search hit, a pin).
  const shownNonce = useRef(focusNonce);
  useEffect(() => {
    if (shownNonce.current === focusNonce || !focusId) return;
    const index = rows.findIndex((row) => row.type === "message" && row.message.id === focusId);
    if (index < 0) return;
    shownNonce.current = focusNonce;
    pinned.current = false;
    settlingUntil.current = 0;
    virtualizer.scrollToIndex(index, { align: "center" });
  });

  // More is loaded a few rows before the reader runs out of them.
  const firstIndex = items[0]?.index ?? 0;
  const lastIndex = items.at(-1)?.index ?? 0;
  const nearStart = opened.current && hasOlder && firstIndex <= 4;
  const nearEnd = opened.current && hasNewer && lastIndex >= rows.length - 5;
  useEffect(() => {
    if (nearStart) onNearStart();
  });
  useEffect(() => {
    if (nearEnd) onNearEnd();
  });

  // Pinned to the end: the reader is following the conversation, so whatever changes the layout
  // (rows being measured, a picture arriving, the composer growing, the keyboard opening) must leave
  // the newest message in view. Where the list is scrolled to decides whether it is pinned; while it
  // is still finding its end after opening, only the reader's own hand can take it away from there.
  useEffect(() => {
    const element = scroller.current;
    const sizer = content.current;
    if (!element || !sizer) return;
    let handUntil = 0;
    const touched = () => {
      handUntil = performance.now() + 2500;
      settlingUntil.current = 0;
    };
    const onScroll = () => {
      const atEnd = virtualizer.isAtEnd(END_THRESHOLD);
      const now = performance.now();
      if (!atEnd && pinned.current && now < settlingUntil.current && now >= handUntil) {
        requestAnimationFrame(() => pinned.current && virtualizer.scrollToEnd());
        return;
      }
      pinned.current = atEnd;
    };
    const observer = new ResizeObserver(() => {
      if (pinned.current) virtualizer.scrollToEnd();
    });
    const gestures = ["wheel", "touchstart", "touchmove", "pointerdown", "keydown"] as const;
    for (const type of gestures) element.addEventListener(type, touched, { passive: true });
    element.addEventListener("scroll", onScroll, { passive: true });
    observer.observe(element);
    observer.observe(sizer);
    return () => {
      for (const type of gestures) element.removeEventListener(type, touched);
      element.removeEventListener("scroll", onScroll);
      observer.disconnect();
    };
  }, [virtualizer]);

  const atEnd = !hasNewer && (rows.length === 0 || virtualizer.isAtEnd(END_THRESHOLD));
  const reported = useRef<boolean | null>(null);
  useEffect(() => {
    if (reported.current === atEnd) return;
    reported.current = atEnd;
    onAtEndChange(atEnd);
  });

  return (
    <div ref={scroller} className="h-full overflow-y-auto overscroll-contain" data-testid="message-list" data-at-end={atEnd} data-ready={ready || undefined}>
      {hasOlder && firstIndex === 0 && (
        <div className="absolute inset-x-0 top-2 z-10 grid place-items-center">
          <Spinner className="size-5 text-ink-faint" />
        </div>
      )}
      <div ref={content} className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
        {items.map((item) => {
          const row = rows[item.index];
          if (!row) return null;
          return (
            <div key={item.key} data-index={item.index} ref={virtualizer.measureElement} className="absolute top-0 left-0 w-full" style={{ transform: `translateY(${item.start}px)` }}>
              {row.type === "day" ? (
                <p className="py-2 text-center">
                  <span className="rounded-full bg-surface-2 px-3 py-1 text-xs font-bold text-ink-soft">{dayLabel(row.at)}</span>
                </p>
              ) : row.type === "unread" ? (
                <p className="flex items-center gap-3 px-4 py-2 text-xs font-black text-pink" data-testid="unread-divider">
                  <span className="h-0.5 flex-1 rounded-full bg-pink/40" />
                  {t("message.unreadFrom")}
                  <span className="h-0.5 flex-1 rounded-full bg-pink/40" />
                </p>
              ) : (
                <MessageItem row={row} highlighted={row.message.id === focusId} onOpenLocation={onOpenLocation} />
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
