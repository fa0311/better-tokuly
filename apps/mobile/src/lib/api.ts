import {
  type Api,
  Code,
  ConnectError,
  createApi,
  identity,
  type RefreshTokenStore,
} from "@tokuly/api";
import { fetch as expoFetch } from "expo/fetch";
import { activeTokenStore } from "#/features/session/vault";
import { API_URL } from "#/lib/config";
import i18n from "#/lib/i18n";
import { apiClientKind } from "#/platform/device";

let instance: Api | null = null;
/** Goes up with every `resetApi()`: an API of an older generation belongs to an account that left. */
let generation = 0;

/**
 * The API of the account that is active on this device. A session object
 * stands for one account: after a switch (or a sign-in beside another
 * account) `resetApi()` drops it, and the next call builds the API of
 * whoever is active now. `expo/fetch` streams, which server-streaming RPCs need.
 */
export function api(): Api {
  instance ??= activeApi(generation);
  return instance;
}

/**
 * The active account's API of one generation. The active token store is the
 * same storage whoever is active, so an API that outlived its account (a
 * report or an upload that goes on late) would refresh with the next
 * account's refresh token — and the server, seeing that token come back
 * spent, would revoke the next account's session (REFRESH_TOKEN_REUSE).
 * Once `resetApi()` ran, that API never touches the token store again and
 * never refreshes: a call goes out only with the access token it still
 * holds (a leave hook that outlasted its wait, `#/seams/lifecycle`, still
 * speaks as its own account), any other is refused (`isAccountLeft`).
 */
function activeApi(own: number): Api {
  const left = () => own !== generation;
  const whileCurrent = async <T>(work: () => Promise<T>): Promise<T> => {
    if (left()) throw accountLeft();
    return work();
  };
  const store: RefreshTokenStore = {
    get: () => whileCurrent(() => activeTokenStore.get()),
    set: (token) => whileCurrent(() => activeTokenStore.set(token)),
    clear: () => whileCurrent(() => activeTokenStore.clear()),
  };
  const fetch = ((input: string, init?: RequestInit) => {
    // A refresh carries no access token; nor does a call made once the held one ran out.
    if (
      left() &&
      !(new Headers(init?.headers).has("authorization") && held.session.peekAccessToken())
    )
      return Promise.reject(accountLeft());
    return withoutCookies(input, init);
  }) as typeof globalThis.fetch;
  const held = apiWith(store, fetch);
  return held;
}

/** The API as the session whose refresh token `store` keeps (the active one, or a parked one). */
export function apiWith(store: RefreshTokenStore, fetch = withoutCookies): Api {
  return createApi({
    baseUrl: API_URL,
    client: apiClientKind,
    refreshTokenStore: store,
    fetch,
    language: () => i18n.language,
  });
}

const ACCOUNT_LEFT = "ACCOUNT_LEFT";

function accountLeft(): ConnectError {
  return new ConnectError("the account left this device", Code.Aborted, {
    "x-tokuly-error": ACCOUNT_LEFT,
  });
}

/** A request of an account that is no longer the active one here, refused before it went out. */
export function isAccountLeft(error: unknown): boolean {
  return ConnectError.from(error).metadata.get("x-tokuly-error") === ACCOUNT_LEFT;
}

/**
 * `expo/fetch` without cookies. The app proves who it is with its tokens
 * alone; on Android its requests would otherwise share the cookie store of
 * the web views (`features/web`), and carry a web session's cookie.
 */
const withoutCookies = ((input: string, init?: RequestInit) =>
  expoFetch(input, {
    ...(init as object),
    credentials: "omit",
  })) as unknown as typeof globalThis.fetch;

/** The active account's API goes: everything still holding it is refused from now on. */
export function resetApi(): void {
  generation += 1;
  instance = null;
}

// Typed clients, created lazily. Features add their own next to their code.
export const authClient = () => api().service(identity.AuthService);
export const profileClient = () => api().service(identity.ProfileService);
