import { openBrowserAsync } from "expo-web-browser";
import { webLink } from "#/lib/config";

/**
 * Opens the web app's settings in the in-app browser, for what is not native
 * yet. The browser has its own sign-in: the backend has no flow that hands a
 * session to the web safely (a short-lived one-time code exchanged for the
 * cookie), and a session token never goes into a URL. So the web asks to
 * sign in, then returns to its settings.
 */
export function openWebSettings(): Promise<unknown> {
  return openBrowserAsync(webLink(`/login?redirect=${encodeURIComponent("/settings")}`));
}
