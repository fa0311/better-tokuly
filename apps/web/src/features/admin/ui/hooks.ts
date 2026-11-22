import { useEffect, useState, useSyncExternalStore } from "react";

/** Whether a CSS media query matches right now (console routes only render in the browser). */
export function useMediaQuery(query: string): boolean {
  return useSyncExternalStore(
    (notify) => {
      const media = window.matchMedia(query);
      media.addEventListener("change", notify);
      return () => media.removeEventListener("change", notify);
    },
    () => window.matchMedia(query).matches,
    () => false,
  );
}

/** Tables turn into cards below this width. */
export const useIsWide = () => useMediaQuery("(min-width: 1024px)");

/** Room for a pane beside a two-column page (the queue next to a case). */
export const useIsExtraWide = () => useMediaQuery("(min-width: 1536px)");

/** `value`, but only after it stopped changing for `delayMs` (search-as-you-type). */
export function useDebounced<T>(value: T, delayMs = 300): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value), delayMs);
    return () => clearTimeout(timer);
  }, [value, delayMs]);
  return debounced;
}

/**
 * Search-as-you-type for a list whose filters live in the address: `text` is
 * what the field shows, `query` (debounced) is what the list asks the server
 * for, and `commit` hands the text over to be written to the address once the
 * typing is done (the field calls it on Enter and on blur).
 *
 * The address is deliberately not moved while typing: a navigation started
 * from a timer could cancel the one a click on a result has just begun.
 */
export function useSearchText(
  applied: string | undefined,
  onCommit: (text: string | undefined) => void,
) {
  const [text, setText] = useState(applied ?? "");
  const query = useDebounced(text.trim()) || undefined;
  const commit = () => {
    const next = text.trim() || undefined;
    if (next !== applied) onCommit(next);
  };
  return { text, setText, query, commit };
}
