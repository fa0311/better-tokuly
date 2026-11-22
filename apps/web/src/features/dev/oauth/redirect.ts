/**
 * Leaving the consent page for the application.
 *
 * The only address the page ever goes to is the `redirect_url` the server
 * returns from `GetConsent`, `ApproveAuthorization` or `DenyAuthorization`:
 * the server has compared it with the application's registered redirect URIs.
 * Nothing from the page's own query string is used as a destination.
 */

/** Schemes that run or show content in this origin instead of leaving it. */
const REFUSED_SCHEMES = new Set(["javascript:", "data:", "vbscript:", "blob:", "file:", "about:"]);

/**
 * The server's redirect as a URL to navigate to, or `undefined` when it is
 * not one a browser should follow. Registered redirect URIs are https, http
 * on a loopback address, or an application's private scheme; a script or
 * data URL is refused whatever the server says.
 */
export function redirectTarget(redirectUrl: string): string | undefined {
  let url: URL;
  try {
    url = new URL(redirectUrl);
  } catch {
    // Not absolute: it would resolve against this page and stay here.
    return undefined;
  }
  return REFUSED_SCHEMES.has(url.protocol) ? undefined : url.href;
}

/** Sends the browser to the application. `false` when the address is refused. */
export function leaveFor(redirectUrl: string): boolean {
  const target = redirectTarget(redirectUrl);
  if (!target) return false;
  window.location.assign(target);
  return true;
}

/** Whether the page is shown inside another page's frame (it must never be). */
export function isFramed(): boolean {
  try {
    return window.top !== window.self;
  } catch {
    // Reading `top` across origins can throw: then we are certainly framed.
    return true;
  }
}

/** The OAuth parameters of an authorization request, exactly as they arrived. */
export interface AuthorizationParams {
  clientId: string;
  redirectUri: string;
  responseType: string;
  scope: string;
  state: string;
  codeChallenge: string;
  codeChallengeMethod: string;
}

/**
 * The authorization request in a query string. The values are passed to the
 * server untouched and decide nothing here: there is no parameter that
 * approves, skips the screen or names where to go afterwards.
 */
export function authorizationParams(query: URLSearchParams): AuthorizationParams {
  const text = (name: string) => query.get(name) ?? "";
  return {
    clientId: text("client_id"),
    redirectUri: text("redirect_uri"),
    responseType: text("response_type"),
    scope: text("scope"),
    state: text("state"),
    codeChallenge: text("code_challenge"),
    codeChallengeMethod: text("code_challenge_method"),
  };
}

/** Marks a query string carried as one value (see `carryQuery`). */
const CARRIED = "q.";

/**
 * A query string as one opaque value, for the way through sign-in and back.
 * The router reads search values as JSON where it can, which would turn a
 * `state` such as `1e3` into `1000`; one value that is never JSON survives.
 */
export function carryQuery(search: string): string {
  const bytes = new TextEncoder().encode(search.replace(/^\?/, ""));
  const base64 = btoa(String.fromCharCode(...bytes));
  return CARRIED + base64.replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

/** The query string a page was opened with: its own, or the one carried in `request`. */
export function carriedQuery(search: string): URLSearchParams {
  const query = new URLSearchParams(search);
  const carried = query.get("request");
  if (!carried?.startsWith(CARRIED)) return query;
  try {
    const base64 = carried.slice(CARRIED.length).replaceAll("-", "+").replaceAll("_", "/");
    const bytes = Uint8Array.from(atob(base64), (char) => char.charCodeAt(0));
    return new URLSearchParams(new TextDecoder().decode(bytes));
  } catch {
    return query;
  }
}
