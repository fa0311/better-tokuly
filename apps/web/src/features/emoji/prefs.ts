import { useSyncExternalStore } from "react";

// Small browser-side facts the emoji components share: whether pictures may move, and whether sensitive emoji are shown.

export const SENSITIVE_KEY = "tokuly.emoji.showSensitive";
const listeners = new Set<() => void>();
let watching = false;

function notify(): void {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  if (!watching) {
    watching = true;
    window.matchMedia("(prefers-reduced-motion: reduce)").addEventListener("change", notify);
    // ドパガキ度 0 means "no movement" too; it lives in an attribute of <html>.
    new MutationObserver(notify).observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-dopa"],
    });
    window.addEventListener("storage", notify);
  }
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function motionAllowed(): boolean {
  return (
    !window.matchMedia("(prefers-reduced-motion: reduce)").matches &&
    document.documentElement.dataset.dopa !== "0"
  );
}

/** False when the person asked for less movement (the system setting, or ドパガキ度 0): animated emoji show their still. */
export function useMotionAllowed(): boolean {
  return useSyncExternalStore(subscribe, motionAllowed, () => false);
}

function showSensitive(): boolean {
  try {
    return window.localStorage.getItem(SENSITIVE_KEY) === "1";
  } catch {
    return false;
  }
}

/** Whether emoji flagged as sensitive are shown as they are (they are blurred otherwise). Kept per browser. */
export function useShowSensitive(): boolean {
  return useSyncExternalStore(subscribe, showSensitive, () => false);
}

export function setShowSensitive(show: boolean): void {
  try {
    if (show) window.localStorage.setItem(SENSITIVE_KEY, "1");
    else window.localStorage.removeItem(SENSITIVE_KEY);
  } catch {
    // Private mode: the choice lasts until the page is left.
  }
  notify();
}

// ───────────────────────────── Is it on screen? ─────────────────────────────

const visibility = new Map<Element, (visible: boolean) => void>();
let observer: IntersectionObserver | undefined;

/**
 * Tells `onChange` whether `element` is on screen (or about to be). One
 * observer serves every animated emoji of the page.
 */
export function watchVisibility(
  element: Element,
  onChange: (visible: boolean) => void,
): () => void {
  observer ??= new IntersectionObserver(
    (entries) => {
      for (const entry of entries) visibility.get(entry.target)?.(entry.isIntersecting);
    },
    { rootMargin: "64px" },
  );
  visibility.set(element, onChange);
  observer.observe(element);
  return () => {
    visibility.delete(element);
    observer?.unobserve(element);
  };
}
