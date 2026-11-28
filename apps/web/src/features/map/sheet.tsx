import { ChevronLeft, X } from "lucide-react";
import { type ReactNode, useEffect, useId, useRef, useSyncExternalStore } from "react";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { useMapUi } from "./ui-store";

const WIDE_QUERY = "(min-width: 768px)";

/** Wide screens get the side panel; phones get bottom sheets. */
export function useIsWide(): boolean {
  return useSyncExternalStore(
    (notify) => {
      const media = window.matchMedia(WIDE_QUERY);
      media.addEventListener("change", notify);
      return () => media.removeEventListener("change", notify);
    },
    () => window.matchMedia(WIDE_QUERY).matches,
    () => false,
  );
}

/**
 * Tells the map how much of it an element covers on one side, so that
 * "centre on me" means the centre of what is still visible. The size is
 * measured from the map's edge to the element's far edge.
 */
export function useCoveredInset<T extends HTMLElement>(
  side: "top" | "bottom" | "left",
  enabled = true,
) {
  const ref = useRef<T>(null);
  const id = useId();
  useEffect(() => {
    const element = ref.current;
    const frame = element?.closest<HTMLElement>("[data-map-frame]");
    if (!element || !frame || !enabled) return;
    const measure = () => {
      // Layout offsets, not the painted box: a sheet sliding in must report where it will rest.
      let left = 0;
      let top = 0;
      for (
        let node: HTMLElement | null = element;
        node && node !== frame;
        node = node.offsetParent as HTMLElement | null
      ) {
        left += node.offsetLeft;
        top += node.offsetTop;
      }
      const size =
        side === "bottom"
          ? frame.clientHeight - top
          : side === "top"
            ? top + element.offsetHeight
            : left + element.offsetWidth;
      useMapUi.getState().setInset(id, side, Math.max(0, Math.round(size)));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    observer.observe(frame);
    return () => {
      observer.disconnect();
      useMapUi.getState().setInset(id, side, null);
    };
  }, [id, side, enabled]);
  return ref;
}

/** The control that opened the sheet, to give the focus back to when it closes. */
let opener: HTMLElement | null = null;

export interface MapSheetProps {
  title: ReactNode;
  subtitle?: ReactNode;
  /** Shown before the title (a face, an emoji). */
  leading?: ReactNode;
  onBack?(): void;
  onClose?(): void;
  /** Stays visible under the scrolling content. */
  footer?: ReactNode;
  children: ReactNode;
  testId?: string;
}

/**
 * The container of everything that opens over the map: a sheet rising from
 * the bottom on phones (above the tab bar), a panel along the left edge on
 * wide screens.
 */
export function MapSheet({
  title,
  subtitle,
  leading,
  onBack,
  onClose,
  footer,
  children,
  testId,
}: MapSheetProps) {
  const { t } = useTranslation("common");
  const wide = useIsWide();
  const ref = useCoveredInset<HTMLElement>(wide ? "left" : "bottom");
  const dismiss = onClose ?? onBack;
  // A sheet someone opened takes the focus, like a dialog would; the always-there
  // friend list on wide screens (nothing to close) leaves it where it is.
  const takesFocus = Boolean(onClose);

  useEffect(() => {
    const sheet = ref.current;
    if (!takesFocus || !sheet) return;
    const active = document.activeElement as HTMLElement | null;
    // Remember what opened the first sheet; sheets opened from inside a sheet keep that origin.
    if (active && active !== document.body && !active.closest("[data-map-sheet]")) opener = active;
    sheet.focus({ preventScroll: true });
    return () => {
      // Once the last sheet is gone the focus goes back to where it came from.
      setTimeout(() => {
        if (document.querySelector("[data-map-sheet]")) return;
        const lost = !document.activeElement || document.activeElement === document.body;
        if (lost && opener?.isConnected) opener.focus({ preventScroll: true });
        opener = null;
      }, 0);
    };
  }, [takesFocus, ref]);

  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-0 z-30 flex px-2 pb-[5.25rem] md:inset-y-0 md:right-auto md:w-[25.5rem] md:p-4">
      <section
        ref={ref}
        data-testid={testId ?? "map-sheet"}
        data-map-sheet
        tabIndex={-1}
        aria-label={typeof title === "string" ? title : undefined}
        onKeyDown={(event) => {
          // Esc closes the sheet — unless it was meant for a dialog or menu opened from it,
          // whose key events bubble here through React although they live elsewhere in the page.
          if (event.key !== "Escape" || !dismiss || event.defaultPrevented) return;
          if (!event.currentTarget.contains(event.target as Node)) return;
          event.stopPropagation();
          dismiss();
        }}
        className="glass sticker-lg pointer-events-auto flex max-h-[52dvh] outline-none w-full animate-slide-up flex-col overflow-hidden rounded-[2rem] md:max-h-none md:h-full"
      >
        <header className="flex items-center gap-2 px-4 pt-4 pb-2">
          {onBack && (
            <button
              type="button"
              onClick={onBack}
              aria-label={t("actions.back")}
              className="grid size-9 shrink-0 place-items-center rounded-full hover:bg-surface-2"
            >
              <ChevronLeft className="size-5" />
            </button>
          )}
          {leading}
          <div className="min-w-0 flex-1">
            <h2 className="truncate text-lg leading-tight font-black">{title}</h2>
            {subtitle && <p className="truncate text-xs font-bold text-ink-soft">{subtitle}</p>}
          </div>
          {onClose && (
            <button
              type="button"
              onClick={onClose}
              aria-label={t("actions.close")}
              className="grid size-9 shrink-0 place-items-center rounded-full hover:bg-surface-2"
            >
              <X className="size-5" />
            </button>
          )}
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-4 pb-4">
          {children}
        </div>
        {footer && <footer className="border-t-2 border-line px-4 py-3">{footer}</footer>}
      </section>
    </div>
  );
}

/** A titled block inside a sheet. */
export function SheetSection({
  title,
  action,
  children,
  className,
}: {
  title?: ReactNode;
  action?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("mt-4 first:mt-1", className)}>
      {(title || action) && (
        <div className="mb-1.5 flex items-center gap-2">
          <h3 className="min-w-0 flex-1 truncate text-xs font-black tracking-wide text-ink-soft">
            {title}
          </h3>
          {action}
        </div>
      )}
      {children}
    </section>
  );
}

/** A small fact as a pill ("バッテリー 42%", "ここに2時間"). */
export function Fact({
  children,
  tone = "plain",
}: {
  children: ReactNode;
  tone?: "plain" | "warn" | "good";
}) {
  return (
    <li
      className={cn(
        "flex items-center gap-1 rounded-full px-3 py-1 text-xs font-bold",
        tone === "warn"
          ? "bg-danger/15 text-danger"
          : tone === "good"
            ? "bg-mint/40 text-ink"
            : "bg-surface-2 text-ink-soft",
      )}
    >
      {children}
    </li>
  );
}

/** "Nothing here yet", sized for a sheet. */
export function SheetEmpty({
  emoji,
  title,
  body,
}: {
  emoji: string;
  title: string;
  body?: string;
}) {
  return (
    <div className="flex items-center gap-3 rounded-2xl border-2 border-dashed border-line px-4 py-3">
      <span className="text-3xl" aria-hidden>
        {emoji}
      </span>
      <div className="min-w-0 flex-1">
        <p className="text-sm font-black">{title}</p>
        {body && <p className="text-xs leading-relaxed text-ink-soft">{body}</p>}
      </div>
    </div>
  );
}
