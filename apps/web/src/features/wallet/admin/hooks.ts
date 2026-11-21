import { useEffect, useState, useSyncExternalStore } from "react";
import { useMe } from "#/stores/session";

/** Whether a CSS media query matches right now (the staff page only renders in the browser). */
function useMediaQuery(query: string): boolean {
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

/** Room for a list and its detail side by side. */
export const useIsWide = () => useMediaQuery("(min-width: 1024px)");

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
 * What the signed-in account may do with other people's wallets. The API
 * decides for real: payout review and lookups need `moderator`, balance
 * adjustments and the coin shop need `admin`.
 */
export function useWalletStaffAccess(): { moderator: boolean; admin: boolean } {
  const roles = useMe()?.roles ?? [];
  const admin = roles.includes("admin");
  return { admin, moderator: admin || roles.includes("moderator") };
}
