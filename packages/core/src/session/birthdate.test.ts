import { create, fromDate, identity } from "@tokuly/api";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  asksForBirthdate,
  asksForBirthdateNow,
  isTooYoung,
  tooYoungUntil,
  watchTooYoung,
} from "./birthdate";

describe("asksForBirthdate", () => {
  it("asks an account without a birthdate", () => {
    expect(asksForBirthdate(create(identity.MeSchema, { birthdate: "" }))).toBe(true);
  });

  it("does not ask an account that has one, nor a bot", () => {
    expect(asksForBirthdate(create(identity.MeSchema, { birthdate: "2010-04-02" }))).toBe(false);
    expect(asksForBirthdate(create(identity.MeSchema, { bot: true }))).toBe(false);
  });
});

describe("asksForBirthdateNow", () => {
  const without = (id: string) => create(identity.MeSchema, { user: { id }, birthdate: "" });

  it("waits while the account that said later is signed in", () => {
    expect(asksForBirthdateNow(without("u1"), null)).toBe(true);
    expect(asksForBirthdateNow(without("u1"), "u1")).toBe(false);
  });

  it("asks another account, and nobody while signed out", () => {
    expect(asksForBirthdateNow(without("u2"), "u1")).toBe(true);
    expect(asksForBirthdateNow(null, null)).toBe(false);
  });
});

describe("isTooYoung", () => {
  it("follows the account, and nobody is while signed out", () => {
    expect(isTooYoung(create(identity.MeSchema, { birthdate: "2015-01-01", tooYoung: true }))).toBe(
      true,
    );
    expect(isTooYoung(create(identity.MeSchema, { birthdate: "" }))).toBe(false);
    expect(isTooYoung(null)).toBe(false);
  });
});

describe("tooYoungUntil", () => {
  it("is the moment the server named, and only while too young", () => {
    const until = fromDate(new Date("2026-12-01T00:00:00Z"));
    expect(tooYoungUntil(create(identity.MeSchema, { tooYoung: true, tooYoungUntil: until }))).toBe(
      Date.parse("2026-12-01T00:00:00Z"),
    );
    expect(tooYoungUntil(create(identity.MeSchema, { tooYoungUntil: until }))).toBe(0);
    expect(tooYoungUntil(null)).toBe(0);
  });
});

describe("watchTooYoung", () => {
  const HOUR = 3600_000;
  const birthday = new Date("2026-12-01T00:00:00Z");
  const young = create(identity.MeSchema, { tooYoung: true, tooYoungUntil: fromDate(birthday) });
  let wake: (() => void) | null = null;
  const onWake = (listener: () => void) => {
    wake = listener;
    return () => {
      wake = null;
    };
  };

  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(birthday.getTime() - 30 * 24 * HOUR));
  });
  afterEach(() => void vi.useRealTimers());

  it("asks again on the birthday, though it is weeks away", async () => {
    const recheck = vi.fn(async () => {});
    watchTooYoung(tooYoungUntil(young), recheck, onWake);
    await vi.advanceTimersByTimeAsync(30 * 24 * HOUR - 1000);
    expect(recheck).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1000);
    expect(recheck).toHaveBeenCalledTimes(1);
  });

  it("asks at once when a sleeping device wakes after it", async () => {
    const recheck = vi.fn(async () => {});
    watchTooYoung(tooYoungUntil(young), recheck, onWake);
    wake?.();
    expect(recheck).not.toHaveBeenCalled();
    // The timers stood still while the device slept.
    vi.setSystemTime(new Date(birthday.getTime() + 2 * HOUR));
    wake?.();
    expect(recheck).toHaveBeenCalledTimes(1);
  });

  it("keeps asking, calmly, while the server still says no", async () => {
    vi.setSystemTime(birthday);
    const recheck = vi.fn(async () => {});
    watchTooYoung(tooYoungUntil(young), recheck, onWake);
    await vi.advanceTimersByTimeAsync(0);
    expect(recheck).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(59_000);
    expect(recheck).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1000);
    expect(recheck).toHaveBeenCalledTimes(2);
  });

  it("does nothing for an account that is not too young, nor after it stops", async () => {
    const recheck = vi.fn(async () => {});
    watchTooYoung(tooYoungUntil(create(identity.MeSchema, {})), recheck, onWake);
    const stop = watchTooYoung(tooYoungUntil(young), recheck, onWake);
    stop();
    expect(wake).toBeNull();
    await vi.advanceTimersByTimeAsync(31 * 24 * HOUR);
    expect(recheck).not.toHaveBeenCalled();
  });
});
