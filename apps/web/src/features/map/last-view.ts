import { useEffect } from "react";
import { onBeforeSignOut } from "#/stores/session";

/** Where the map was left, so it reopens there. */
export const LAST_VIEW_KEY = "tokuly.map.view";

/**
 * Where someone last looked at the map is, more often than not, where they
 * live. It leaves the browser when they sign out: the next person at a
 * shared computer opens the map somewhere neutral.
 */
export function useForgetLastViewOnSignOut(): void {
  useEffect(() => onBeforeSignOut(() => window.localStorage.removeItem(LAST_VIEW_KEY)), []);
}
