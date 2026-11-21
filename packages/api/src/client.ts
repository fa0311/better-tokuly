import { type Client, createClient, type Interceptor, type Transport } from "@connectrpc/connect";
import { createConnectTransport } from "@connectrpc/connect-web";
import type { DescService } from "@bufbuild/protobuf";
import { isStaleToken } from "./errors.js";
import { type RefreshTokenStore, Session } from "./session.js";

export interface ApiOptions {
  /** Origin of the API, e.g. `https://tokuly.example` or `http://localhost:8081`. */
  baseUrl: string;
  /**
   * `web` makes the server keep the refresh token in an HttpOnly cookie.
   * Native clients pass a `refreshTokenStore` instead.
   */
  client: "web" | "ios" | "android" | "desktop";
  refreshTokenStore?: RefreshTokenStore;
  /** Custom fetch (React Native streaming fetch, tests). Defaults to the global one. */
  fetch?: typeof globalThis.fetch;
  /** Preferred language sent as `Accept-Language`. */
  language?: () => string;
}

export interface Api {
  readonly session: Session;
  readonly transport: Transport;
  /** A typed client for any generated service: `api.service(FriendService)`. */
  service<T extends DescService>(service: T): Client<T>;
}

/**
 * Creates the API entry point: an authenticated Connect transport plus the
 * session that feeds it tokens.
 *
 * Every call carries the current access token. A call rejected because the
 * token went stale is retried once after a refresh, so callers never see
 * token expiry.
 */
export function createApi(options: ApiOptions): Api {
  const baseFetch = options.fetch ?? globalThis.fetch.bind(globalThis);
  const withCredentials: typeof globalThis.fetch = (input, init) =>
    baseFetch(input, { ...init, credentials: "include" });

  const common: Interceptor = (next) => (request) => {
    if (options.client === "web") request.header.set("x-tokuly-client", "web");
    const language = options.language?.();
    if (language) request.header.set("accept-language", language);
    return next(request);
  };

  const bare = createConnectTransport({
    baseUrl: options.baseUrl,
    useBinaryFormat: true,
    fetch: withCredentials,
    interceptors: [common],
  });
  const session = new Session(bare, options.refreshTokenStore);

  const authenticate: Interceptor = (next) => async (request) => {
    const token = await session.accessToken();
    if (token) request.header.set("authorization", `Bearer ${token}`);
    try {
      return await next(request);
    } catch (error) {
      // Streams cannot be replayed; unary calls are retried once.
      if (request.stream || !isStaleToken(error)) throw error;
      if (!(await session.refresh())) throw error;
      const fresh = session.peekAccessToken();
      if (fresh) request.header.set("authorization", `Bearer ${fresh}`);
      return next(request);
    }
  };

  const transport = createConnectTransport({
    baseUrl: options.baseUrl,
    useBinaryFormat: true,
    fetch: withCredentials,
    interceptors: [common, authenticate],
  });

  const clients = new Map<DescService, Client<DescService>>();
  return {
    session,
    transport,
    service<T extends DescService>(service: T): Client<T> {
      let client = clients.get(service);
      if (!client) {
        client = createClient(service, transport) as Client<DescService>;
        clients.set(service, client);
      }
      return client as Client<T>;
    },
  };
}
