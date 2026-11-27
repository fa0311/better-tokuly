import { type Client, Code, ConnectError, createClient, type Transport } from "@connectrpc/connect";
import { isRateLimited, isUnauthenticated, retryAfterMs } from "./errors.js";
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
/** Told when the session starts to belong to another account (or another session of it). */
type AccountChangeListener = (next: { userId: string; sessionId: string }) => void;

/** Refresh this long before the access token actually expires. */
const REFRESH_MARGIN_MS = 60_000;

/**
 * How `restore()` waits on a rate limit: `unnamed` when the server names no
 * wait, never less than `shortest` or more than `longest` at a time, and
 * `inAll` at most (two of the server's one-minute windows) before it gives up.
 */
const RATE_LIMIT_WAIT_MS = { unnamed: 5_000, shortest: 1_000, longest: 60_000, inAll: 120_000 };

/** What a refresh came to: adopted or not, or rate-limited and to be tried again in so long. */
type Refreshed = boolean | { retryInMs: number };

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
  private refreshing: Promise<Refreshed> | null = null;
  private restoring: Promise<boolean> | null = null;
  private current: SessionState = { status: "unknown" };
  private readonly listeners = new Set<Listener>();
  private readonly accountListeners = new Set<AccountChangeListener>();
  /**
   * Set once a refresh came back as another account: this session object
   * still stands for the previous one (its screen, caches, subscriptions),
   * so it never sends anything again — not as the old account, not as the
   * new one. The app starts over (a web page reloads).
   */
  private stale = false;
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

  /**
   * Subscribes to the session moving to another account: a refresh that
   * came back as someone else (another tab switched the active account of
   * this browser) or a switch made here. Everything held for the previous
   * account (caches, subscriptions, the screen) is stale then; a web client
   * reloads. Returns an unsubscribe function.
   */
  onAccountChange(listener: AccountChangeListener): () => void {
    this.accountListeners.add(listener);
    return () => this.accountListeners.delete(listener);
  }

  /**
   * Adopts the tokens of a successful sign-in / sign-up / refresh. Resolves
   * to false, adopting nothing, when they belong to another account (or
   * another session) than the one signed in here: the tokens are dropped,
   * the session goes stale (see `onAccountChange`) and nothing is sent.
   */
  async adopt(response: AuthResponse): Promise<boolean> {
    if (this.stale || !response.accessToken || !response.me) return false;
    const previous = this.current;
    const userId = response.me.user?.id ?? "";
    const changed =
      previous.status === "signedIn" &&
      (previous.me.user?.id !== userId || previous.sessionId !== response.sessionId);
    if (changed) {
      this.stale = true;
      this.accessTokenValue = null;
      this.expiresAtMs = 0;
      for (const listener of this.accountListeners)
        listener({ userId, sessionId: response.sessionId });
      return false;
    }
    this.accessTokenValue = response.accessToken;
    this.expiresAtMs = toMillis(response.accessTokenExpiresAt);
    if (response.refreshToken && this.store) await this.store.set(response.refreshToken);
    this.setState({ status: "signedIn", me: response.me, sessionId: response.sessionId });
    return true;
  }

  /** The session went stale: another account became the active one. */
  get isStale(): boolean {
    return this.stale;
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

  /**
   * Tries to resume a previous session. Resolves to whether it worked. A
   * rate-limited refresh (a school opening the app after the bell) says
   * nothing about the session: the state stays unknown, and the refresh is
   * tried again after the wait the server named. Concurrent callers share
   * one attempt, and so one wait. A server that keeps limiting is given up
   * on like one that cannot be reached: signed out here, the refresh token
   * kept, so the person gets a screen and the next start resumes.
   */
  restore(): Promise<boolean> {
    this.restoring ??= this.doRestore().finally(() => {
      this.restoring = null;
    });
    return this.restoring;
  }

  private async doRestore(): Promise<boolean> {
    let refreshed = await this.refreshOnce();
    let waitedMs = 0;
    while (
      typeof refreshed === "object" &&
      waitedMs + refreshed.retryInMs <= RATE_LIMIT_WAIT_MS.inAll
    ) {
      const { retryInMs } = refreshed;
      await new Promise((resolve) => setTimeout(resolve, retryInMs));
      waitedMs += retryInMs;
      refreshed = await this.refreshOnce();
    }
    const resumed = refreshed === true;
    if (!resumed && this.current.status === "unknown") this.setState({ status: "signedOut" });
    return resumed;
  }

  /**
   * Exchanges the refresh token for a new access token. Concurrent callers
   * share one request. Resolves to false when the session is gone, or the
   * refresh did not go through this time.
   */
  async refresh(): Promise<boolean> {
    return (await this.refreshOnce()) === true;
  }

  private refreshOnce(): Promise<Refreshed> {
    if (this.stale) return Promise.resolve(false);
    this.refreshing ??= this.doRefresh().finally(() => {
      this.refreshing = null;
    });
    return this.refreshing;
  }

  private async doRefresh(): Promise<Refreshed> {
    try {
      const refreshToken = this.store ? ((await this.store.get()) ?? "") : "";
      if (this.store && !refreshToken) {
        this.forget();
        return false;
      }
      const response = await this.auth.refreshToken({ refreshToken });
      return await this.adopt(response);
    } catch (error) {
      // Only a definite rejection signs the user out; a network failure keeps
      // the session so the app can retry when it is back online.
      if (isUnauthenticated(error)) {
        await this.store?.clear();
        this.forget();
      }
      if (isRateLimited(error)) {
        const wait = retryAfterMs(error) ?? RATE_LIMIT_WAIT_MS.unnamed;
        const { shortest, longest } = RATE_LIMIT_WAIT_MS;
        return { retryInMs: Math.min(Math.max(wait, shortest), longest) };
      }
      return false;
    }
  }

  private forget(): void {
    this.accessTokenValue = null;
    this.expiresAtMs = 0;
    this.setState({ status: "signedOut" });
  }

  /**
   * A valid access token, or `null` when signed out. Throws (code
   * `aborted`) once the session is stale: a request must not go out as an
   * account the screen does not show.
   */
  async accessToken(): Promise<string | null> {
    this.throwIfStale();
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
    this.throwIfStale();
    return this.accessTokenValue;
  }

  private throwIfStale(): void {
    if (this.stale)
      throw new ConnectError("another account became active on this device", Code.Aborted);
  }

  /** The access token if one is cached, without refreshing. */
  peekAccessToken(): string | null {
    if (this.stale) return null;
    return this.accessTokenValue && Date.now() < this.expiresAtMs ? this.accessTokenValue : null;
  }

  /**
   * Signs out on the server and locally. `allAccountsOnDevice`: every
   * account of this browser too (a shared computer; native clients sign
   * their parked accounts out with the tokens they hold).
   */
  async signOut(allSessions = false, allAccountsOnDevice = false): Promise<void> {
    const token = this.peekAccessToken();
    try {
      await this.auth.signOut(
        { allSessions, allAccountsOnDevice },
        { headers: token ? { authorization: `Bearer ${token}` } : undefined },
      );
    } catch {
      // Signing out locally must always succeed.
    }
    await this.store?.clear();
    this.forget();
  }
}
