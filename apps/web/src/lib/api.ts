import { type Api, createApi, identity } from "@tokuly/api";
import i18n from "#/lib/i18n";

/**
 * The single API instance of the web app.
 *
 * The browser always talks to its own origin: the edge proxy (or the dev
 * server's proxy rule) forwards `/api/*` to the backend, so the refresh cookie
 * is first-party.
 */
export const API_PATH = "/api";
let instance: Api | null = null;

export function api(): Api {
  if (!instance) {
    if (typeof window === "undefined") throw new Error("the API client is browser-only");
    instance = createApi({
      baseUrl: `${window.location.origin}${API_PATH}`,
      client: "web",
      language: () => i18n.language,
    });
  }
  return instance;
}

// Typed clients, created lazily. Feature modules add their own next to their code.
export const authClient = () => api().service(identity.AuthService);
export const accountClient = () => api().service(identity.AccountService);
export const profileClient = () => api().service(identity.ProfileService);
