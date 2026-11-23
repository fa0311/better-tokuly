import { type notify, toMillis } from "@tokuly/api";
import { MailCheck, MoreHorizontal, Trash2 } from "lucide-react";
import { type MouseEvent, type PointerEvent, type ReactNode, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { Menu } from "#/components/ui/menu";
import { cn } from "#/lib/cn";
import { NotificationFace } from "./notification-face";
import { notificationPath, resolveTargetPath } from "./target";
import { formatWhen } from "./when";

/** One inbox entry: tap to open, swipe left (touch) or use the menu to delete. */
export function NotificationRow({
  entry,
  now,
  onOpen,
  onRead,
  onDelete,
}: {
  entry: notify.Notification;
  now: number;
  /** Mark read and go to the entry's target. */
  onOpen(): void;
  onRead(): void;
  onDelete(): void;
}) {
  const { t, i18n } = useTranslation("notifications");
  const swipe = useSwipeAway(onDelete);
  const target = resolveTargetPath(notificationPath(entry.kind, entry.path));
  const more = entry.groupCount - 1;

  const body: ReactNode = (
    <>
      <NotificationFace entry={entry} />
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1.5">
          {!entry.read && (
            <span className="size-2.5 shrink-0 rounded-full bg-pink">
              <span className="sr-only">{t("row.unread")}</span>
            </span>
          )}
          <span className={cn("min-w-0 truncate", entry.read ? "font-bold" : "font-black")}>
            {entry.title}
          </span>
        </span>
        {entry.body && (
          <span
            className={cn(
              "mt-0.5 line-clamp-2 text-sm leading-snug",
              entry.read ? "text-ink-soft" : "text-ink",
            )}
          >
            {entry.body}
          </span>
        )}
        <span className="mt-1.5 flex flex-wrap items-center gap-1.5 text-xs font-bold text-ink-faint">
          <time dateTime={new Date(toMillis(entry.occurredAt)).toISOString()}>
            {formatWhen(toMillis(entry.occurredAt), now, i18n.language)}
          </time>
          {more > 0 && (
            <Chip className="h-5 bg-surface px-2 text-[11px]">
              {t("row.grouped", { count: more })}
            </Chip>
          )}
          {entry.urgent && (
            <Chip className="h-5 bg-coral px-2 text-[11px] text-white">{t("row.urgent")}</Chip>
          )}
        </span>
      </span>
      {entry.imageUrl && (
        <img
          src={entry.imageUrl}
          alt=""
          loading="lazy"
          decoding="async"
          className="size-14 shrink-0 rounded-xl border-2 border-line object-cover"
        />
      )}
    </>
  );
  const action = "flex min-w-0 flex-1 items-start gap-3 rounded-xl p-1.5 text-left";

  // A real link where the entry leads into the app, so it can also be opened in a new tab.
  const openInPlace = (event: MouseEvent) => {
    if (event.metaKey || event.ctrlKey || event.shiftKey || event.button !== 0) {
      onRead();
      return;
    }
    event.preventDefault();
    onOpen();
  };

  return (
    <li className="relative overflow-hidden rounded-2xl" data-unread={entry.read ? undefined : ""}>
      {swipe.offset < 0 && (
        <div
          aria-hidden
          className="absolute inset-0 flex items-center justify-end gap-2 rounded-2xl bg-danger pr-5 font-bold text-white"
        >
          <Trash2 className="size-5" />
          {t("row.delete")}
        </div>
      )}
      <div
        {...swipe.handlers}
        style={{
          transform: swipe.offset ? `translateX(${swipe.offset}px)` : undefined,
          transition: swipe.dragging ? "none" : "transform 180ms ease-out",
        }}
        className={cn(
          "relative flex touch-pan-y items-start gap-1 rounded-2xl p-1.5",
          entry.read ? "bg-surface" : "bg-brand-soft",
          entry.urgent && !entry.read && "ring-2 ring-coral ring-inset",
        )}
      >
        {target?.type === "internal" ? (
          <a href={target.href} onClick={openInPlace} className={action} draggable={false}>
            {body}
          </a>
        ) : (
          <button type="button" onClick={onOpen} className={action}>
            {body}
          </button>
        )}
        <Menu
          trigger={
            <Button
              variant="ghost"
              size="icon-sm"
              className="mt-1 shrink-0 text-ink-soft"
              aria-label={t("row.menu")}
            >
              <MoreHorizontal className="size-5" />
            </Button>
          }
          actions={[
            ...(entry.read
              ? []
              : [{ label: t("row.markRead"), icon: <MailCheck />, onSelect: onRead }]),
            { label: t("row.delete"), icon: <Trash2 />, danger: true, onSelect: onDelete },
          ]}
        />
      </div>
    </li>
  );
}

/** Past this much of a leftward drag (or a third of the row), letting go deletes. */
const COMMIT_PX = 96;
/** Movement before a touch counts as a drag and its direction is decided. */
const SLOP_PX = 8;

/**
 * Swipe-left-to-dismiss for touch and pen (a mouse has the menu). The row
 * follows the finger; vertical movement is left to the page's scrolling.
 */
function useSwipeAway(onCommit: () => void) {
  const [offset, setOffset] = useState(0);
  const [dragging, setDragging] = useState(false);
  const gesture = useRef<{
    pointerId: number;
    x: number;
    y: number;
    width: number;
    horizontal: boolean | null;
  } | null>(null);
  const swallowClick = useRef(false);

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    swallowClick.current = false;
    if (event.pointerType === "mouse") return;
    gesture.current = {
      pointerId: event.pointerId,
      x: event.clientX,
      y: event.clientY,
      width: event.currentTarget.offsetWidth,
      horizontal: null,
    };
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const current = gesture.current;
    if (!current || current.pointerId !== event.pointerId) return;
    const dx = event.clientX - current.x;
    const dy = event.clientY - current.y;
    if (current.horizontal === null) {
      if (Math.abs(dx) < SLOP_PX && Math.abs(dy) < SLOP_PX) return;
      current.horizontal = Math.abs(dx) > Math.abs(dy);
      if (current.horizontal) {
        swallowClick.current = true;
        setDragging(true);
        try {
          event.currentTarget.setPointerCapture(event.pointerId);
        } catch {
          // Not an active pointer (synthetic event): the drag still works inside the row.
        }
      }
    }
    if (current.horizontal) setOffset(Math.min(0, dx));
  };

  const end = (event: PointerEvent<HTMLDivElement>, cancelled: boolean) => {
    const current = gesture.current;
    if (!current || current.pointerId !== event.pointerId) return;
    gesture.current = null;
    if (!current.horizontal) return;
    setDragging(false);
    const dx = event.clientX - current.x;
    if (!cancelled && dx <= -Math.min(COMMIT_PX, current.width / 3)) {
      setOffset(-current.width);
      window.setTimeout(onCommit, 180);
    } else {
      setOffset(0);
    }
  };

  return {
    offset,
    dragging,
    handlers: {
      onPointerDown,
      onPointerMove,
      onPointerUp: (event: PointerEvent<HTMLDivElement>) => end(event, false),
      onPointerCancel: (event: PointerEvent<HTMLDivElement>) => end(event, true),
      // The tap that ends a swipe must not open the entry.
      onClickCapture: (event: MouseEvent) => {
        if (!swallowClick.current) return;
        swallowClick.current = false;
        event.preventDefault();
        event.stopPropagation();
      },
    },
  };
}
