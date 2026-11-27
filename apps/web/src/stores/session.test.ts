import type { SessionState } from "@tokuly/api";
import { describe, expect, it, vi } from "vitest";

/** The API's session, told by the test what it becomes. */
const fake = vi.hoisted(() => {
  const listeners = new Set<(state: SessionState) => void>();
  return {
    become(state: SessionState) {
      for (const listener of listeners) listener(state);
    },
    session: {
      subscribe: (listener: (state: SessionState) => void) => {
        listeners.add(listener);
        return () => listeners.delete(listener);
      },
      onAccountChange: () => () => {},
      restore: async () => true,
      assumeSignedOut: () => {},
      signOut: async () => {
        for (const listener of listeners) listener({ status: "signedOut" });
      },
    },
  };
});
vi.mock("#/lib/api", () => ({ api: () => ({ session: fake.session }) }));
const forget = vi.hoisted(() => vi.fn(async () => {}));
vi.mock("#/seams/sign-out", () => ({ forgetSignedOutAccount: forget }));
const stored = new Map<string, string>([["tokuly.signedIn", "1"]]);
vi.stubGlobal("window", {
  localStorage: {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: (key: string, value: string) => void stored.set(key, value),
    removeItem: (key: string) => void stored.delete(key),
  },
});
vi.stubGlobal("document", { cookie: "" });
vi.stubGlobal(
  "BroadcastChannel",
  class {
    onmessage: unknown = null;
  },
);

const { signOut, startSession } = await import("./session");

const signedIn: SessionState = {
  status: "signedIn",
  sessionId: "s1",
  me: { user: { id: "u1" } } as never,
};

describe("the session store", () => {
  it("forgets the account once it was signed in and signs out, not on a start that finds nobody", () => {
    startSession();
    // An offline start: signed out here, its session kept for later. Its drafts stay.
    fake.become({ status: "signedOut" });
    expect(forget).not.toHaveBeenCalled();

    fake.become(signedIn);
    fake.become({ status: "signedOut" });
    expect(forget).toHaveBeenCalledOnce();
  });

  it("signs out only once nothing of the account is left: the next screen comes after it", async () => {
    startSession();
    fake.become(signedIn);
    let cleaned!: () => void;
    forget.mockImplementationOnce(() => new Promise<void>((resolve) => (cleaned = resolve)));

    let done = false;
    const signingOut = signOut().then(() => (done = true));
    await vi.waitFor(() => expect(cleaned).toBeDefined());
    await Promise.resolve();
    expect(done).toBe(false);
    cleaned();
    await signingOut;
    expect(done).toBe(true);
  });
});
