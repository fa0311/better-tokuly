import { useEffect } from "react";
import { api } from "#/lib/api";

/** Where the map was left, so it reopens there. */
export const LAST_VIEW_KEY = "tokuly.map.view";

/**
 * Where someone last looked at the map is, more often than not, where they
 * live. It leaves the browser when the session ends, however it ends:
 * signed out here, from another device or by the server. The next person at
 * a shared computer opens the map somewhere neutral.
 */
export function useForgetLastViewOnSignOut(): void {
  useEffect(
    () =>
      api().session.subscribe((state) => {
        if (state.status === "signedOut") window.localStorage.removeItem(LAST_VIEW_KEY);
      }),
    [],
  );
}
