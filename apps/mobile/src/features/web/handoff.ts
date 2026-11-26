import { identity } from "@tokuly/api";
import { CryptoDigestAlgorithm, digestStringAsync, getRandomBytes } from "expo-crypto";
import { type Account, ensureActive } from "#/features/session/account";
import { SERVER_URL } from "#/lib/config";

/** The header the web view's first load carries the verifier in (identity's `handoff::VERIFIER_HEADER`). */
export const VERIFIER_HEADER = "x-tokuly-handoff-verifier";

/**
 * A web screen opened in the app: the hand-off URL that signs the web view
 * in, for the account that asked, and the verifier its first load sends.
 * `sessionId` keys the web view, so another account never gets this one's.
 */
export interface WebSession {
  sessionId: string;
  url: string;
  verifier: string;
}

const hex = (bytes: Uint8Array) =>
  Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");

/**
 * Asks the server for a one-time code that opens `path` on the web signed in
 * as `account` (`CreateWebHandoff`; 60 seconds, once). The account is
 * captured by the caller before anything awaits and checked again when the
 * answer is back: if another account took over meanwhile (a switch, a
 * sign-out), the code is thrown away (`AccountChanged`), never loaded. The
 * session token itself never leaves the app: the web view only ever sees
 * this URL.
 *
 * The code is bound to a verifier made here (256 random bits; the server
 * keeps its SHA-256): the URL alone signs nobody in, only the web view's
 * first load, which sends the verifier in a header, does. A code that leaks
 * cannot sign someone else's browser in as this account.
 */
export async function requestWebSession(account: Account, path: string): Promise<WebSession> {
  const verifier = hex(getRandomBytes(32));
  const verifierHash = await digestStringAsync(CryptoDigestAlgorithm.SHA256, verifier);
  const { url } = await account.api
    .service(identity.AuthService)
    .createWebHandoff({ path, verifierHash });
  ensureActive(account);
  newest = { sessionId: account.sessionId, url: `${SERVER_URL}${url}`, verifier };
  return newest;
}

/** The hand-off asked for last. */
let newest: WebSession | null = null;

/**
 * Whether `session` has been replaced: the server keeps one web session per
 * app session, the newest hand-off's, so the web view of an earlier one is
 * signed out as soon as a later one loads.
 */
export function isSuperseded(session: WebSession): boolean {
  return session !== newest;
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
