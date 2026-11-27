import { create } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { Code, ConnectError, createRouterTransport } from "@connectrpc/connect";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  type AuthResponse,
  AuthResponseSchema,
  AuthService,
} from "./gen/tokuly/identity/v1/identity_pb.js";
import { type RefreshTokenStore, Session } from "./session.js";

const MINUTE = 60_000;

/** The tokens the server hands out for `userId`'s session, valid for `validMs`. */
function tokens(userId: string, sessionId = "s1", validMs = 15 * MINUTE): AuthResponse {
  return create(AuthResponseSchema, {
    accessToken: `at-${userId}-${Date.now()}`,
    accessTokenExpiresAt: timestampFromMs(Date.now() + validMs),
    refreshToken: `rt-${userId}-${Date.now()}`,
    sessionId,
    me: { user: { id: userId } },
  });
}

/** A phone's secure storage. */
function memoryStore(
  token: string | null = "rt-start",
): RefreshTokenStore & { token: string | null } {
  const store = {
    token,
    get: async () => store.token,
    set: async (next: string) => {
      store.token = next;
    },
    clear: async () => {
      store.token = null;
    },
  };
  return store;
}

/** The auth service: each refresh answers with what `answer` says; everything sent is kept. */
function server() {
  const fake = {
    refreshes: [] as string[],
    signOuts: [] as { allSessions: boolean; authorization: string | null }[],
    answer: (): Promise<AuthResponse> => Promise.resolve(tokens("alice")),
    signOutFails: false,
  };
  const transport = createRouterTransport(({ service }) =>
    service(AuthService, {
      refreshToken: (request) => {
        fake.refreshes.push(request.refreshToken);
        return fake.answer();
      },
      signOut: (request, context) => {
        fake.signOuts.push({
          allSessions: request.allSessions,
          authorization: context.requestHeader.get("authorization"),
        });
        if (fake.signOutFails) throw new ConnectError("down", Code.Unavailable);
        return {};
      },
    }),
  );
  return { fake, transport };
}

/** A promise and the hands that settle it. */
function later<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const rateLimited = (seconds?: number) =>
  new ConnectError(
    "slow down",
    Code.ResourceExhausted,
    seconds === undefined ? {} : { "retry-after": String(seconds) },
  );

describe("Session", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  describe("resuming", () => {
    it("signs in with the stored refresh token and keeps the one it is given", async () => {
      const { fake, transport } = server();
      const store = memoryStore();
      const session = new Session(transport, store);
      expect(session.state.status).toBe("unknown");

      expect(await session.restore()).toBe(true);
      expect(fake.refreshes).toEqual(["rt-start"]);
      expect(session.state).toMatchObject({ status: "signedIn", sessionId: "s1" });
      expect(store.token).toMatch(/^rt-alice-/);
    });

    it("is signed out without asking when the device holds no token", async () => {
      const { fake, transport } = server();
      const session = new Session(transport, memoryStore(null));
      expect(await session.restore()).toBe(false);
      expect(fake.refreshes).toEqual([]);
      expect(session.state.status).toBe("signedOut");
    });

    it("forgets a token the server refused", async () => {
      const { fake, transport } = server();
      fake.answer = () => Promise.reject(new ConnectError("no", Code.Unauthenticated));
      const store = memoryStore();
      const session = new Session(transport, store);
      expect(await session.restore()).toBe(false);
      expect(session.state.status).toBe("signedOut");
      expect(store.token).toBeNull();
    });

    it("keeps the token when the server cannot be reached, for the next start", async () => {
      const { fake, transport } = server();
      fake.answer = () => Promise.reject(new ConnectError("offline", Code.Unavailable));
      const store = memoryStore();
      const session = new Session(transport, store);
      expect(await session.restore()).toBe(false);
      expect(session.state.status).toBe("signedOut");
      expect(store.token).toBe("rt-start");
    });

    it("waits out a rate limit as the server names it, with every caller on one attempt", async () => {
      const { fake, transport } = server();
      fake.answer = () => Promise.reject(rateLimited(3));
      const session = new Session(transport, memoryStore());
      const first = session.restore();
      const second = session.restore();
      await vi.advanceTimersByTimeAsync(0);
      expect(fake.refreshes).toHaveLength(1);
      // Rate-limited says nothing about the session: still unknown, not signed out.
      expect(session.state.status).toBe("unknown");

      fake.answer = () => Promise.resolve(tokens("alice"));
      await vi.advanceTimersByTimeAsync(2_999);
      expect(fake.refreshes).toHaveLength(1);
      await vi.advanceTimersByTimeAsync(1);
      expect(await first).toBe(true);
      expect(await second).toBe(true);
      expect(fake.refreshes).toHaveLength(2);
    });

    it("gives a server that keeps limiting up after two minutes, keeping the token", async () => {
      const { fake, transport } = server();
      fake.answer = () => Promise.reject(rateLimited(60));
      const store = memoryStore();
      const session = new Session(transport, store);
      const resumed = session.restore();
      await vi.advanceTimersByTimeAsync(120_000);
      expect(await resumed).toBe(false);
      expect(fake.refreshes).toHaveLength(3);
      expect(session.state.status).toBe("signedOut");
      expect(store.token).toBe("rt-start");
    });

    it("lets a sign-in made during a rate-limit wait stand", async () => {
      const { fake, transport } = server();
      fake.answer = () => Promise.reject(rateLimited(10));
      const session = new Session(transport, memoryStore());
      const resumed = session.restore();
      await vi.advanceTimersByTimeAsync(0);
      expect(await session.adopt(tokens("bob", "s2"))).toBe(true);
      await vi.advanceTimersByTimeAsync(10_000);
      expect(await resumed).toBe(true);
      expect(fake.refreshes).toHaveLength(1);
      expect(session.state).toMatchObject({ status: "signedIn", sessionId: "s2" });
    });
  });

  describe("the access token", () => {
    it("is refreshed once for every caller that needs one at the same time", async () => {
      const { fake, transport } = server();
      const answer = later<AuthResponse>();
      fake.answer = () => answer.promise;
      const session = new Session(transport, memoryStore());
      const asked = [session.accessToken(), session.accessToken(), session.refresh()];
      await vi.advanceTimersByTimeAsync(0);
      const response = tokens("alice");
      answer.resolve(response);
      expect(await Promise.all(asked)).toEqual([response.accessToken, response.accessToken, true]);
      expect(fake.refreshes).toHaveLength(1);
    });

    it("is handed out as it is while valid, and refreshed behind the scenes in its last minute", async () => {
      const { fake, transport } = server();
      const session = new Session(transport, memoryStore());
      const first = tokens("alice", "s1", 5 * MINUTE);
      await session.adopt(first);

      expect(await session.accessToken()).toBe(first.accessToken);
      expect(fake.refreshes).toHaveLength(0);

      vi.advanceTimersByTime(4 * MINUTE + 1);
      // Still valid: used now, a new one is on its way.
      expect(await session.accessToken()).toBe(first.accessToken);
      await vi.advanceTimersByTimeAsync(0);
      expect(fake.refreshes).toHaveLength(1);
      expect(await session.accessToken()).not.toBe(first.accessToken);
    });

    it("is null once signed out", async () => {
      const { transport } = server();
      const session = new Session(transport, memoryStore(null));
      await session.restore();
      expect(await session.accessToken()).toBeNull();
      expect(session.peekAccessToken()).toBeNull();
    });
  });

  describe("another account", () => {
    it("a refresh that comes back as someone else is not adopted, and nothing is sent again", async () => {
      const { fake, transport } = server();
      const store = memoryStore();
      const session = new Session(transport, store);
      await session.restore();
      const aliceToken = store.token;
      const changes: unknown[] = [];
      session.onAccountChange((next) => changes.push(next));
      const states: string[] = [];
      session.subscribe((state) => states.push(state.status));

      // Another tab of this browser switched to Bob.
      fake.answer = () => Promise.resolve(tokens("bob", "s2"));
      expect(await session.refresh()).toBe(false);
      expect(changes).toEqual([{ userId: "bob", sessionId: "s2" }]);
      expect(session.isStale).toBe(true);
      // Still Alice's screen, and none of Bob's tokens kept for it.
      expect(session.state).toMatchObject({ status: "signedIn", sessionId: "s1" });
      expect(states).toEqual([]);
      expect(store.token).toBe(aliceToken);
      expect(session.peekAccessToken()).toBeNull();
      await expect(session.accessToken()).rejects.toMatchObject({ code: Code.Aborted });

      // Nothing more goes out, as either account.
      expect(await session.refresh()).toBe(false);
      expect(await session.adopt(tokens("alice"))).toBe(false);
      expect(fake.refreshes).toHaveLength(2);
    });

    it("another session of the same person counts as another account", async () => {
      const { fake, transport } = server();
      const session = new Session(transport, memoryStore());
      await session.restore();
      const changes: unknown[] = [];
      session.onAccountChange((next) => changes.push(next));
      fake.answer = () => Promise.resolve(tokens("alice", "s9"));
      expect(await session.refresh()).toBe(false);
      expect(changes).toEqual([{ userId: "alice", sessionId: "s9" }]);
      expect(session.isStale).toBe(true);
    });

    it("a refresh of the same session is no change", async () => {
      const { transport } = server();
      const session = new Session(transport, memoryStore());
      await session.restore();
      const changes = vi.fn();
      session.onAccountChange(changes);
      expect(await session.refresh()).toBe(true);
      expect(changes).not.toHaveBeenCalled();
      expect(session.isStale).toBe(false);
    });
  });

  describe("signing out", () => {
    it("tells the server with the token it holds, and forgets everything", async () => {
      const { fake, transport } = server();
      const store = memoryStore();
      const session = new Session(transport, store);
      await session.restore();
      const token = session.peekAccessToken();
      await session.signOut(true);
      expect(fake.signOuts).toEqual([{ allSessions: true, authorization: `Bearer ${token}` }]);
      expect(session.state.status).toBe("signedOut");
      expect(store.token).toBeNull();
      expect(await session.accessToken()).toBeNull();
    });

    it("signs out here even when the server cannot be reached", async () => {
      const { fake, transport } = server();
      fake.signOutFails = true;
      const store = memoryStore();
      const session = new Session(transport, store);
      await session.restore();
      await session.signOut();
      expect(session.state.status).toBe("signedOut");
      expect(store.token).toBeNull();
    });

    it("does not let a refresh that was on its way sign the account back in", async () => {
      const { fake, transport } = server();
      const store = memoryStore();
      const session = new Session(transport, store);
      await session.restore();
      const answer = later<AuthResponse>();
      fake.answer = () => answer.promise;
      const refreshed = session.refresh();
      await vi.advanceTimersByTimeAsync(0);

      await session.signOut();
      answer.resolve(tokens("alice"));
      expect(await refreshed).toBe(false);
      expect(session.state.status).toBe("signedOut");
      expect(store.token).toBeNull();
      expect(session.peekAccessToken()).toBeNull();
    });

    it("does not let the refusal of a refresh from before a sign-out end the account signed in since", async () => {
      const { fake, transport } = server();
      const store = memoryStore();
      const session = new Session(transport, store);
      await session.restore();
      const answer = later<AuthResponse>();
      fake.answer = () => answer.promise;
      const refreshed = session.refresh();
      await vi.advanceTimersByTimeAsync(0);

      await session.signOut();
      expect(await session.adopt(tokens("bob", "s2"))).toBe(true);
      answer.reject(new ConnectError("that session ended", Code.Unauthenticated));
      expect(await refreshed).toBe(false);
      expect(session.state).toMatchObject({ status: "signedIn", sessionId: "s2" });
      expect(store.token).toMatch(/^rt-bob-/);
    });
  });
});
