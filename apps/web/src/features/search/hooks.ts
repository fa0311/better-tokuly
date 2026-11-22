import { useEffect, useState, useSyncExternalStore } from "react";

/** `value`, but only after it stopped changing for `delayMs` (typeahead). */
export function useDebounced<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value), delayMs);
    return () => clearTimeout(timer);
  }, [value, delayMs]);
  return debounced;
}

/** Whether a CSS media query matches right now (search only renders in the browser). */
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

/** A pointer that hovers and a keyboard at hand: where shortcuts are worth advertising. */
export const useHasKeyboard = () => useMediaQuery("(hover: hover) and (pointer: fine)");
