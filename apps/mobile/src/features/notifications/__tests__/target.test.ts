import { SERVER_URL } from "#/lib/config";
import { INBOX, pushRoute, targetOf } from "../target";

describe("where a server path goes", () => {
  it("opens a screen the app has", () => {
    expect(pushRoute("/chat/42")).toBe("/chat/42");
    expect(pushRoute(`${SERVER_URL}/u/alice`)).toBe("/u/alice");
    expect(pushRoute("/notes/abc")).toBe("/notes/abc");
    expect(pushRoute("/spaces/xyz")).toBe("/spaces/xyz");
    expect(pushRoute("/hangouts/h1")).toBe("/spaces/hangouts/h1");
  });

  it("opens what only the web has in the web view, signed in, not the inbox", () => {
    expect(pushRoute("/communities/xyz")).toBe(
      `/web?path=${encodeURIComponent("/communities/xyz")}`,
    );
    expect(targetOf("/notes/channels/abc")).toEqual({
      type: "screen",
      href: `/web?path=${encodeURIComponent("/notes/channels/abc")}`,
    });
    expect(pushRoute("/settings/data")).toBe(`/web?path=${encodeURIComponent("/settings/data")}`);
    expect(pushRoute("/settings/privacy")).toBe("/settings/privacy");
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
