import { type Api, createApi, identity, type RefreshTokenStore } from "@tokuly/api";
import { fetch as expoFetch } from "expo/fetch";
import { activeTokenStore } from "#/features/session/vault";
import { API_URL } from "#/lib/config";
import i18n from "#/lib/i18n";
import { apiClientKind } from "#/platform/device";

let instance: Api | null = null;

/**
 * The API of the account that is active on this device. A session object
 * stands for one account: after a switch (or a sign-in beside another
 * account) `resetApi()` drops it, and the next call builds the API of
 * whoever is active now. `expo/fetch` streams, which server-streaming RPCs need.
 */
export function api(): Api {
  instance ??= apiWith(activeTokenStore);
  return instance;
}

/** The API as the session whose refresh token `store` keeps (the active one, or a parked one). */
export function apiWith(store: RefreshTokenStore): Api {
  return createApi({
    baseUrl: API_URL,
    client: apiClientKind,
    refreshTokenStore: store,
    fetch: expoFetch as unknown as typeof globalThis.fetch,
    language: () => i18n.language,
  });
}

export function resetApi(): void {
  instance = null;
}

// Typed clients, created lazily. Features add their own next to their code.
export const authClient = () => api().service(identity.AuthService);
export const profileClient = () => api().service(identity.ProfileService);
