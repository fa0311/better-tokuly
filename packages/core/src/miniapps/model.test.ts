import { describe, expect, it } from "vitest";
import { cardMessage, miniAppHref, nextStep, readMiniAppLink } from "./model";

const ID = "0193b7a2-1c2d-7e3f-8a9b-0c1d2e3f4a5b";
const WEB = "https://tokuly.app";

describe("mini app addresses", () => {
  it("builds the page address with what it opens with", () => {
    expect(miniAppHref(ID)).toBe(`/miniapps/${ID}`);
    expect(
      miniAppHref(ID, {
        path: "/?total=3600",
        conversationId: "c1",
        origin: "https://warikan.example",
      }),
    ).toBe(`/miniapps/${ID}?p=%2F%3Ftotal%3D3600&c=c1&o=https%3A%2F%2Fwarikan.example`);
  });

  it("reads links to mini apps on this site only", () => {
    expect(readMiniAppLink(`${WEB}/miniapps/${ID}?p=%2F%3Ftotal%3D3600`, WEB)).toEqual({
      appId: ID,
      path: "/?total=3600",
    });
    expect(readMiniAppLink(`/miniapps/${ID.toUpperCase()}`, WEB)).toEqual({ appId: ID, path: "" });
    for (const href of [
      `https://evil.example/miniapps/${ID}`,
      `${WEB}/miniapps/not-an-id`,
      `${WEB}/miniapps/${ID}/more`,
      `${WEB}/developers/apps/${ID}`,
      "javascript:alert(1)",
    ])
      expect(readMiniAppLink(href, WEB), href).toBeUndefined();
  });

  it("turns a card into a message ending in its link", () => {
    expect(
      cardMessage(
        { title: "割り勘", text: "1人 ¥1,200", path: "/?total=3600" },
        `${WEB}/chat/x`,
        ID,
      ),
    ).toBe(`割り勘\n1人 ¥1,200\n${WEB}/miniapps/${ID}?p=%2F%3Ftotal%3D3600`);
    expect(cardMessage({ title: "日程", text: "", path: "/" }, WEB, ID)).toBe(
      `日程\n${WEB}/miniapps/${ID}`,
    );
  });
});

describe("nextStep", () => {
  const state = { requested: ["profile"], granted: [] as string[], inConversation: true };

  it("asks once for a requested scope, then runs", () => {
    expect(nextStep({ method: "getProfile" }, state)).toEqual({ kind: "ask", scope: "profile" });
    expect(nextStep({ method: "getProfile" }, { ...state, granted: ["profile"] })).toEqual({
      kind: "run",
    });
  });

  it("refuses scopes the manifest never asked for, whatever was granted", () => {
    expect(
      nextStep({ method: "pickFriends", max: 3 }, { ...state, granted: ["friends"] }),
    ).toMatchObject({
      kind: "refuse",
      code: "SCOPE_NOT_REQUESTED",
    });
  });

  it("shares only into a conversation", () => {
    const share = { method: "shareCard" as const, card: { title: "t", text: "", path: "/" } };
    expect(nextStep(share, state)).toEqual({ kind: "run" });
    expect(nextStep(share, { ...state, inConversation: false })).toMatchObject({
      code: "NOT_IN_CONVERSATION",
    });
  });
});
