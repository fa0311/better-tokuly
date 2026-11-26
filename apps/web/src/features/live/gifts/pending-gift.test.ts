import { describe, expect, it } from "vitest";
import { keepPendingGift, readPendingGift, settlePendingGift } from "./pending-gift";

function memoryStore() {
  const items = new Map<string, string>();
  return {
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => void items.set(key, value),
    removeItem: (key: string) => void items.delete(key),
  };
}

const attempt = { key: "gift-k1-abcdefgh", coins: 12_345n, message: "がんばれ" };

describe("a pending gift", () => {
  it("is kept by account and stream until the server answers it", () => {
    const store = memoryStore();
    keepPendingGift("u1", "s1", attempt, store);
    expect(readPendingGift("u1", "s1", store)).toEqual(attempt);
    expect(readPendingGift("u1", "s2", store)).toBeNull();
    expect(readPendingGift("u2", "s1", store)).toBeNull();
    settlePendingGift("u1", attempt, store);
    expect(readPendingGift("u1", "s1", store)).toBeNull();
  });

  it("is not settled by the answer to another attempt", () => {
    const store = memoryStore();
    keepPendingGift("u1", "s1", attempt, store);
    settlePendingGift("u1", { ...attempt, key: "gift-other-abcdefgh" }, store);
    expect(readPendingGift("u1", "s1", store)?.key).toBe(attempt.key);
  });

  it("works where the browser keeps nothing", () => {
    keepPendingGift("u1", "s1", attempt, null);
    expect(readPendingGift("u1", "s1", null)).toBeNull();
  });
});
