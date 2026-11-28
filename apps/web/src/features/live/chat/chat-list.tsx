import { useVirtualizer } from "@tanstack/react-virtual";
import { live } from "@tokuly/api";
import { ArrowDown } from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { ChatLine } from "./chat-line";

/** Closer to the end than this counts as following the chat. */
const END_THRESHOLD = 48;

function estimate(message: live.ChatMessage | undefined): number {
  if (!message) return 32;
  if (message.kind === live.ChatMessageKind.GIFT) return message.text ? 110 : 70;
  if (message.kind === live.ChatMessageKind.STAMP) return 104;
  return 30 + Math.floor(message.text.length / 24) * 20;
}

/**
 * The scrolling chat. Only the lines on screen exist in the page, so a chat
 * that moves fast stays smooth. It follows the newest line until the reader
 * scrolls up, and then offers the way back down.
 */
export function ChatList({
  messages,
  tiers,
  selfHandle,
  empty,
  onMore,
  onMention,
  onOpenStamp,
  className,
}: {
  /** Oldest first. */
  messages: live.ChatMessage[];
  tiers: live.GiftTier[];
  selfHandle?: string;
  /** Shown while there is nothing to read yet. */
  empty?: ReactNode;
  onMore?(message: live.ChatMessage): void;
  onMention?(message: live.ChatMessage): void;
  onOpenStamp?(stampId: string): void;
  className?: string;
}) {
  // The virtualizer is a mutable object whose answers change without its identity changing,
  // which the React Compiler cannot see: this component is rendered without memoization.
  "use no memo";
  const { t } = useTranslation("live");
  const scroller = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  /** The reader is following the newest lines. */
  const following = useRef(true);
  const [away, setAway] = useState(false);
  const virtualizer = useVirtualizer({
    count: messages.length,
    getScrollElement: () => scroller.current,
    estimateSize: (index) => estimate(messages[index]),
    getItemKey: (index) => messages[index]?.id ?? index,
    overscan: 8,
    paddingStart: 6,
    paddingEnd: 8,
    anchorTo: "end",
    followOnAppend: true,
    scrollEndThreshold: END_THRESHOLD,
  });

  const toEnd = () => {
    following.current = true;
    setAway(false);
    virtualizer.scrollToEnd();
  };

  // Whatever changes the layout while the reader follows (lines being measured, a stamp
  // loading, the composer growing) must leave the newest line in view.
  useEffect(() => {
    const element = scroller.current;
    const sizer = content.current;
    if (!element || !sizer) return;
    const onScroll = () => {
      const atEnd = virtualizer.isAtEnd(END_THRESHOLD);
      following.current = atEnd;
      setAway(!atEnd);
    };
    const observer = new ResizeObserver(() => {
      if (following.current) virtualizer.scrollToEnd();
    });
    element.addEventListener("scroll", onScroll, { passive: true });
    observer.observe(element);
    observer.observe(sizer);
    return () => {
      element.removeEventListener("scroll", onScroll);
      observer.disconnect();
    };
  }, [virtualizer]);

  const newest = messages.at(-1)?.id;
  useEffect(() => {
    if (following.current) virtualizer.scrollToEnd();
  }, [newest, virtualizer]);

  return (
    <div className={cn("relative min-h-0", className)}>
      <div
        ref={scroller}
        className="h-full overflow-y-auto overscroll-contain"
        role="log"
        aria-label={t("chat.title")}
        data-testid="chat-list"
      >
        {messages.length === 0 ? (
          empty
        ) : (
          <div
            ref={content}
            className="relative w-full"
            style={{ height: virtualizer.getTotalSize() }}
          >
            {virtualizer.getVirtualItems().map((item) => {
              const message = messages[item.index];
              if (!message) return null;
              return (
                <div
                  key={item.key}
                  ref={virtualizer.measureElement}
                  data-index={item.index}
                  className="absolute top-0 left-0 w-full"
                  style={{ transform: `translateY(${item.start}px)` }}
                >
                  <ChatLine
                    message={message}
                    tiers={tiers}
                    selfHandle={selfHandle}
                    onMore={onMore}
                    onMention={onMention}
                    onOpenStamp={onOpenStamp}
                  />
                </div>
              );
            })}
          </div>
        )}
      </div>
      {away && messages.length > 0 && (
        <button
          type="button"
          onClick={toEnd}
          className="pressable sticker-sm absolute bottom-2 left-1/2 flex h-9 -translate-x-1/2 items-center gap-1.5 rounded-full bg-surface px-4 text-sm font-black"
        >
          <ArrowDown className="size-4" />
          {t("chat.toNewest")}
        </button>
      )}
    </div>
  );
}
