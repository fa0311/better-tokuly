import { useEffect } from "react";
import { onBeforeLeave } from "#/features/session/session";
import { clearWebData } from "#/platform/web-data";

/**
 * Runs while someone is signed in: before a switch or a sign-out, whatever
 * that account's web screens stored (cookies, localStorage, cache) goes, so
 * the next account's web views start from nothing.
 */
export function WebBackground() {
  useEffect(() => onBeforeLeave(clearWebData), []);
  return null;
}
