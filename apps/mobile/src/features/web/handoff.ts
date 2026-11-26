import { identity } from "@tokuly/api";
import { type Account, ensureActive } from "#/features/media/account";
import { SERVER_URL } from "#/lib/config";

/**
 * A web screen opened in the app: the hand-off URL that signs the web view
 * in, for the account that asked. `sessionId` keys the web view, so another
 * account never gets this one's.
 */
export interface WebSession {
  sessionId: string;
  url: string;
}

/**
 * Asks the server for a one-time code that opens `path` on the web signed in
 * as `account` (`CreateWebHandoff`; 60 seconds, once). The account is
 * captured by the caller before anything awaits and checked again when the
 * answer is back: if another account took over meanwhile (a switch, a
 * sign-out), the code is thrown away (`AccountChanged`), never loaded. The
 * session token itself never leaves the app: the web view only ever sees
 * this URL.
 */
export async function requestWebSession(account: Account, path: string): Promise<WebSession> {
  const { url } = await account.api.service(identity.AuthService).createWebHandoff({ path });
  ensureActive(account);
  return { sessionId: account.sessionId, url: `${SERVER_URL}${url}` };
}

const ORIGIN = new URL(SERVER_URL).origin;

/** Whether the web view may load `url` itself: this server's own pages. Anything else opens in the system browser. */
export function staysInApp(url: string): boolean {
  if (url === "about:blank") return true;
  try {
    return new URL(url).origin === ORIGIN;
  } catch {
    return false;
  }
}

/** The in-app route that shows `path` (a path of the web app) in the web view. */
export function webScreenHref(path: string): string {
  return `/web?path=${encodeURIComponent(path)}`;
}
