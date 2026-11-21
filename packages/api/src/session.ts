import { type Client, createClient, type Transport } from "@connectrpc/connect";
import { isUnauthenticated } from "./errors.js";
import { type AuthResponse, AuthService, type Me } from "./gen/tokuly/identity/v1/identity_pb.js";
import { toMillis } from "./time.js";

/**
 * Where the refresh token lives between app launches.
 *
 * - Web: omit it. The server keeps the refresh token in an HttpOnly cookie.
 * - Native (mobile, desktop): back it with the platform's secure storage.
 */
export interface RefreshTokenStore {
  get(): Promise<string | null>;
  set(token: string): Promise<void>;
  clear(): Promise<void>;
}

export type SessionState =
  | { status: "unknown" }
  | { status: "signedOut" }
  | { status: "signedIn"; me: Me; sessionId: string };

type Listener = (state: SessionState) => void;

/** Refresh this long before the access token actually expires. */
const REFRESH_MARGIN_MS = 60_000;

/**
 * Holds the signed-in state and the short-lived access token.
 *
 * The access token only ever lives in memory. `accessToken()` hands out a
 * valid one, transparently refreshing (single-flight) when it is about to
 * expire; `restore()` re-establishes the session on app start.
 */
export class Session {
  private accessTokenValue: string | null = null;
  private expiresAtMs = 0;
  private refreshing: Promise<boolean> | null = null;
  private current: SessionState = { status: "unknown" };
  private readonly listeners = new Set<Listener>();
  private readonly auth: Client<typeof AuthService>;

  /**
   * @param transport A transport WITHOUT the auth interceptor (refresh must
   *   not recurse into itself).
   */
  constructor(
    transport: Transport,
    private readonly store?: RefreshTokenStore,
  ) {
    this.auth = createClient(AuthService, transport);
  }

  get state(): SessionState {
    return this.current;
  }

  /** Subscribes to state changes; returns an unsubscribe function. */
  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private setState(state: SessionState): void {
    this.current = state;
    for (const listener of this.listeners) listener(state);
  }

  /** Adopts the tokens of a successful sign-in / sign-up / refresh. */
  async adopt(response: AuthResponse): Promise<void> {
    if (!response.accessToken || !response.me) return;
    this.accessTokenValue = response.accessToken;
    this.expiresAtMs = toMillis(response.accessTokenExpiresAt);
    if (response.refreshToken && this.store) await this.store.set(response.refreshToken);
    this.setState({ status: "signedIn", me: response.me, sessionId: response.sessionId });
  }

  /** Replaces the cached account (after a profile update). */
  updateMe(me: Me): void {
    if (this.current.status === "signedIn") this.setState({ ...this.current, me });
  }

  /**
   * Declares that there is no session to resume (e.g. the app knows nobody
   * signed in on this device), skipping the refresh round trip.
   */
  assumeSignedOut(): void {
    if (this.current.status === "unknown") this.setState({ status: "signedOut" });
  }

  /** Tries to resume a previous session. Resolves to whether it worked. */
  async restore(): Promise<boolean> {
    const ok = await this.refresh();
    if (!ok && this.current.status === "unknown") this.setState({ status: "signedOut" });
    return ok;
  }

  /**
   * Exchanges the refresh token for a new access token. Concurrent callers
   * share one request. Resolves to false when the session is gone.
   */
  refresh(): Promise<boolean> {
    this.refreshing ??= this.doRefresh().finally(() => {
      this.refreshing = null;
    });
    return this.refreshing;
  }

  private async doRefresh(): Promise<boolean> {
    try {
      const refreshToken = this.store ? ((await this.store.get()) ?? "") : "";
      if (this.store && !refreshToken) {
        this.forget();
        return false;
      }
      const response = await this.auth.refreshToken({ refreshToken });
      await this.adopt(response);
      return true;
    } catch (error) {
      // Only a definite rejection signs the user out; a network failure keeps
      // the session so the app can retry when it is back online.
      if (isUnauthenticated(error)) {
        await this.store?.clear();
        this.forget();
      }
      return false;
    }
  }

  private forget(): void {
    this.accessTokenValue = null;
    this.expiresAtMs = 0;
    this.setState({ status: "signedOut" });
  }

  /** A valid access token, or `null` when signed out. */
  async accessToken(): Promise<string | null> {
    if (this.accessTokenValue && Date.now() < this.expiresAtMs - REFRESH_MARGIN_MS) {
      return this.accessTokenValue;
    }
    if (this.current.status === "signedOut") return null;
    if (this.accessTokenValue && Date.now() < this.expiresAtMs) {
      // Still valid: refresh in the background, use the current one now.
      void this.refresh();
      return this.accessTokenValue;
    }
    await this.refresh();
    return this.accessTokenValue;
  }

  /** The access token if one is cached, without refreshing. */
  peekAccessToken(): string | null {
    return this.accessTokenValue && Date.now() < this.expiresAtMs ? this.accessTokenValue : null;
  }

  /** Signs out on the server and locally. */
  async signOut(allSessions = false): Promise<void> {
    const token = this.peekAccessToken();
    try {
      await this.auth.signOut(
        { allSessions },
        { headers: token ? { authorization: `Bearer ${token}` } : undefined },
      );
    } catch {
      // Signing out locally must always succeed.
    }
    await this.store?.clear();
    this.forget();
  }
}
