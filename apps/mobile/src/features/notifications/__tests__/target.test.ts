import { SERVER_URL } from "#/lib/config";
import { INBOX, pushRoute, targetOf } from "../target";

describe("where a server path goes", () => {
  it("opens a screen the app has", () => {
    expect(pushRoute("/chat/42")).toBe("/chat/42");
    expect(pushRoute(`${SERVER_URL}/u/alice`)).toBe("/u/alice");
    expect(pushRoute("/notes/abc")).toBe("/notes/abc");
  });

  it("opens what only the web has in the web view, signed in, not the inbox", () => {
    expect(pushRoute("/spaces/xyz")).toBe(`/web?path=${encodeURIComponent("/spaces/xyz")}`);
    expect(targetOf("/notes/channels/abc")).toEqual({
      type: "screen",
      href: `/web?path=${encodeURIComponent("/notes/channels/abc")}`,
    });
    expect(pushRoute("/settings")).toBe(`/web?path=${encodeURIComponent("/settings")}`);
  });

  it("never opens an outside link by itself", () => {
    expect(targetOf("https://example.com/news")).toEqual({
      type: "web",
      url: "https://example.com/news",
    });
    expect(pushRoute("https://example.com/news")).toBe(INBOX);
    expect(pushRoute("javascript:alert(1)")).toBe(INBOX);
    // Not a page of the web app either: nothing claims it.
    expect(pushRoute("/login")).toBe(INBOX);
  });
});
