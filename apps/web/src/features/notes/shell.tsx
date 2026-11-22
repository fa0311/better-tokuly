import { useNavigate, useRouter } from "@tanstack/react-router";
import { ChevronLeft, Feather, RefreshCw } from "lucide-react";
import { createContext, type ReactNode, useContext, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Spinner } from "#/components/ui/spinner";
import { cn } from "#/lib/cn";
import { type ComposeContext, openCompose } from "./compose-store";

/** The scrolling element of a notes page (what a virtualised list measures against) and the name its scroll position is remembered under. */
interface Scroller {
  element: HTMLElement | null;
  scrollId: string;
}

const ScrollerContext = createContext<Scroller>({ element: null, scrollId: "" });

export function useScroller(): Scroller {
  return useContext(ScrollerContext);
}

/** How far the page has to be dragged down before letting go refreshes it. */
const PULL_THRESHOLD = 64;
const PULL_MAX = 96;

/**
 * Pull to refresh for a scrolling element: dragging down from the very top
 * moves an indicator along and, past the threshold, refreshes on release.
 */
function usePullToRefresh(scroller: HTMLElement | null, refresh: (() => Promise<unknown>) | undefined): { distance: number; refreshing: boolean } {
  const [distance, setDistance] = useState(0);
  const [refreshing, setRefreshing] = useState(false);
  const latest = useRef(refresh);
  useEffect(() => {
    latest.current = refresh;
  });
  const enabled = Boolean(refresh);

  useEffect(() => {
    if (!scroller || !enabled) return;
    let startY: number | null = null;
    let pulled = 0;
    const onStart = (event: TouchEvent) => {
      startY = scroller.scrollTop <= 0 && event.touches.length === 1 ? event.touches[0]!.clientY : null;
      pulled = 0;
    };
    const onMove = (event: TouchEvent) => {
      if (startY === null) return;
      const delta = event.touches[0]!.clientY - startY;
      if (delta <= 0 || scroller.scrollTop > 0) {
        pulled = 0;
        setDistance(0);
        return;
      }
      // The page resists: the indicator travels half as far as the finger.
      pulled = Math.min(PULL_MAX, delta / 2);
      setDistance(pulled);
      if (event.cancelable) event.preventDefault();
    };
    const onEnd = () => {
      const reached = pulled >= PULL_THRESHOLD;
      startY = null;
      pulled = 0;
      if (!reached || !latest.current) {
        setDistance(0);
        return;
      }
      setRefreshing(true);
      setDistance(PULL_THRESHOLD);
      void latest.current().finally(() => {
        setRefreshing(false);
        setDistance(0);
      });
    };
    scroller.addEventListener("touchstart", onStart, { passive: true });
    scroller.addEventListener("touchmove", onMove, { passive: false });
    scroller.addEventListener("touchend", onEnd);
    scroller.addEventListener("touchcancel", onEnd);
    return () => {
      scroller.removeEventListener("touchstart", onStart);
      scroller.removeEventListener("touchmove", onMove);
      scroller.removeEventListener("touchend", onEnd);
      scroller.removeEventListener("touchcancel", onEnd);
    };
  }, [scroller, enabled]);

  return { distance, refreshing };
}

export interface NotesPageProps {
  title: ReactNode;
  /**
   * Shows a back button. It returns to where the person came from, or to
   * this path when the page was opened directly.
   */
  back?: string;
  /** Extra controls at the right of the header. */
  actions?: ReactNode;
  /** Under the header, above the columns: tabs. Sticks with the header. */
  toolbar?: ReactNode;
  /** A second column on wide screens (trends); phones do not show it. */
  aside?: ReactNode;
  /** Re-reads what the page shows: wired to pull-to-refresh and to a button in the header. */
  onRefresh?(): Promise<unknown>;
  /** Stable name of the page's scroll position, so going back returns to the same place. */
  scrollId: string;
  children: ReactNode;
}

/**
 * The frame of every notes page: the same sticky header and centred column
 * as other pages, plus what timelines need — the scrolling element (for
 * virtualised lists), pull to refresh, and a second column on wide screens.
 */
export function NotesPage({ title, back, actions, toolbar, aside, onRefresh, scrollId, children }: NotesPageProps) {
  const { t } = useTranslation("notes");
  const router = useRouter();
  const navigate = useNavigate();
  const [scroller, setScroller] = useState<HTMLDivElement | null>(null);
  const [header, setHeader] = useState<HTMLElement | null>(null);
  const [busy, setBusy] = useState(false);
  // Things that stick below the header (the "new notes" pill) need to know how tall it is.
  useEffect(() => {
    if (!scroller || !header) return;
    const measure = () => scroller.style.setProperty("--notes-header", `${header.offsetHeight}px`);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(header);
    return () => observer.disconnect();
  }, [scroller, header]);
  const pull = usePullToRefresh(scroller, onRefresh);
  const column = aside ? "max-w-5xl" : "max-w-2xl";

  const refresh = async () => {
    if (!onRefresh || busy) return;
    setBusy(true);
    try {
      await onRefresh();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div ref={setScroller} data-scroll-restoration-id={scrollId} data-testid="notes-scroller" className="relative h-full overflow-y-auto">
     <ScrollerContext.Provider value={{ element: scroller, scrollId }}>
      <header ref={setHeader} className="glass sticky top-0 z-20 border-b-2 border-line">
        <div className={cn("mx-auto flex h-16 items-center gap-2 px-4", column)}>
          {back && (
            <button
              type="button"
              aria-label={t("shell.back")}
              onClick={() => (router.history.canGoBack() ? router.history.back() : void navigate({ to: back }))}
              className="-ml-1 grid size-9 shrink-0 place-items-center rounded-full hover:bg-surface-2"
            >
              <ChevronLeft className="size-6" />
            </button>
          )}
          <h1 className="min-w-0 flex-1 truncate text-xl font-black">{title}</h1>
          {actions}
          {onRefresh && (
            <button type="button" aria-label={t("shell.refresh")} onClick={() => void refresh()} className="grid size-9 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-2 hover:text-ink">
              <RefreshCw className={cn("size-5", (busy || pull.refreshing) && "animate-spin")} />
            </button>
          )}
        </div>
        {toolbar && <div className={cn("mx-auto px-4 pb-2.5", column)}>{toolbar}</div>}
      </header>

      {pull.distance > 0 && (
        <div aria-hidden className="pointer-events-none sticky top-20 z-10 flex h-0 justify-center">
          <span
            className="sticker-sm grid size-10 place-items-center rounded-full bg-surface text-brand"
            style={{ transform: `translateY(${pull.distance - 28}px) rotate(${pull.distance * 4}deg)`, opacity: Math.min(1, pull.distance / PULL_THRESHOLD) }}
          >
            {pull.refreshing ? <Spinner className="size-5" /> : <RefreshCw className="size-5" />}
          </span>
        </div>
      )}

      <div className={cn("mx-auto px-3 pt-3 pb-32 sm:px-4 md:pb-10", column, aside && "lg:grid lg:grid-cols-[minmax(0,1fr)_19rem] lg:gap-6")}>
        <div className={cn("min-w-0", aside && "mx-auto w-full max-w-2xl lg:mx-0 lg:max-w-none")}>{children}</div>
        {aside && <aside className="hidden lg:block">{aside}</aside>}
      </div>
     </ScrollerContext.Provider>
    </div>
  );
}

function scrollParent(element: HTMLElement): HTMLElement | null {
  for (let node = element.parentElement; node; node = node.parentElement) {
    const overflow = getComputedStyle(node).overflowY;
    if (overflow === "auto" || overflow === "scroll") return node;
  }
  return null;
}

/**
 * Lets a timeline live on a page that is not a `NotesPage` (a profile): it
 * finds the element that scrolls around it and hands it to the lists inside.
 */
export function ScrollScope({ scrollId, children }: { scrollId: string; children: ReactNode }) {
  const [anchor, setAnchor] = useState<HTMLDivElement | null>(null);
  const [element, setElement] = useState<HTMLElement | null>(null);
  useEffect(() => {
    if (anchor) setElement(scrollParent(anchor));
  }, [anchor]);
  return (
    <div ref={setAnchor}>
      <ScrollerContext.Provider value={{ element, scrollId }}>{children}</ScrollerContext.Provider>
    </div>
  );
}

function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
}

/**
 * The floating "write a note" button of notes pages, with its keyboard
 * shortcut: `N` opens the composer whenever nothing is being typed and no
 * dialog is open.
 */
export function ComposeButton({ context }: { context?: ComposeContext }) {
  const { t } = useTranslation("notes");
  const latest = useRef(context);
  useEffect(() => {
    latest.current = context;
  });

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() !== "n" || event.metaKey || event.ctrlKey || event.altKey || event.isComposing) return;
      if (isTyping(event.target) || document.querySelector("[role='dialog'], [role='menu']")) return;
      event.preventDefault();
      openCompose(latest.current);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <button
      type="button"
      aria-label={t("compose.open")}
      aria-keyshortcuts="N"
      title={t("compose.openHint")}
      onClick={() => openCompose(context)}
      className="sticker-lg pressable fixed right-4 bottom-[calc(max(env(safe-area-inset-bottom),0.75rem)+4.75rem)] z-30 grid size-14 place-items-center rounded-full bg-lime text-[#17122b] md:right-8 md:bottom-8 md:size-16"
    >
      <Feather className="size-6 md:size-7" strokeWidth={2.5} />
    </button>
  );
}
