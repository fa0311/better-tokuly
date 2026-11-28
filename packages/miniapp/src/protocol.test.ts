import { describe, expect, it } from "vitest";
import {
  cardTarget,
  isCardPath,
  PROTOCOL,
  readRequest,
  readResponse,
  scopeOf,
} from "./protocol.ts";

const request = (call: unknown, id: unknown = "1") => ({ protocol: PROTOCOL, id, call });

describe("readRequest", () => {
  it("ignores what is not a request of the protocol", () => {
    for (const data of [
      null,
      "hello",
      { method: "getProfile" },
      { protocol: "other", id: "1", call: { method: "hello" } },
      request({ method: "hello" }, ""),
      request({ method: "hello" }, 7),
      request({ method: "hello" }, "x".repeat(65)),
    ])
      expect(readRequest(data)).toBeUndefined();
  });

  it("reads the calls", () => {
    expect(readRequest(request({ method: "getProfile", extra: true }))).toEqual({
      id: "1",
      call: { method: "getProfile" },
    });
    expect(readRequest(request({ method: "pickFriends" }))).toEqual({
      id: "1",
      call: { method: "pickFriends", max: 20 },
    });
    expect(
      readRequest(
        request({ method: "shareCard", card: { title: " 割り勘 ", text: "1人 ¥1,200" } }),
      ),
    ).toEqual({
      id: "1",
      call: { method: "shareCard", card: { title: "割り勘", text: "1人 ¥1,200", path: "/" } },
    });
  });

  it("refuses requests that break a rule", () => {
    const code = (call: unknown) => {
      const read = readRequest(request(call));
      return read && "error" in read ? read.error.code : undefined;
    };
    expect(code({ method: "location" })).toBe("UNKNOWN_METHOD");
    expect(code({})).toBe("INVALID_REQUEST");
    for (const max of [0, 21, 1.5, "3"])
      expect(code({ method: "pickFriends", max })).toBe("INVALID_REQUEST");
    for (const card of [
      undefined,
      { title: "" },
      { title: "x".repeat(61) },
      { title: "two\nlines" },
      { title: "ok", text: "y".repeat(201) },
      { title: "ok", path: "https://evil.example/" },
      { title: "ok", path: "//evil.example/" },
      { title: "ok", path: "javascript:alert(1)" },
    ])
      expect(code({ method: "shareCard", card }), JSON.stringify(card)).toBe("INVALID_REQUEST");
  });
});

describe("scopes and cards", () => {
  it("names the scope a call needs", () => {
    expect(scopeOf({ method: "getProfile" })).toBe("profile");
    expect(scopeOf({ method: "pickFriends", max: 3 })).toBe("friends");
    expect(
      scopeOf({ method: "shareCard", card: { title: "t", text: "", path: "/" } }),
    ).toBeUndefined();
  });

  it("keeps a card on the mini app's origin", () => {
    const launch = "https://warikan.example/app/?from=tokuly";
    expect(cardTarget(launch, "/?total=3600")).toBe("https://warikan.example/?total=3600");
    expect(cardTarget(launch, "/app/result#top")).toBe("https://warikan.example/app/result#top");
    for (const path of ["//evil.example/", "/\\evil.example", "https://evil.example/", "app"])
      expect(cardTarget(launch, path), path).toBeUndefined();
    expect(isCardPath(`/${"a".repeat(500)}`)).toBe(false);
  });
});

describe("readResponse", () => {
  it("reads answers and refusals of the protocol only", () => {
    expect(readResponse({ protocol: PROTOCOL, id: "1", ok: true, result: 3 })).toMatchObject({
      ok: true,
      result: 3,
    });
    expect(
      readResponse({
        protocol: PROTOCOL,
        id: "1",
        ok: false,
        error: { code: "CANCELLED", message: "no" },
      }),
    ).toMatchObject({ ok: false, error: { code: "CANCELLED" } });
    expect(readResponse({ protocol: PROTOCOL, id: "1" })).toBeUndefined();
    expect(readResponse({ id: "1", ok: true })).toBeUndefined();
  });
});
