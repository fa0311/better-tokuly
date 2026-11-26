import { SERVER_URL } from "#/lib/config";
import { INBOX, pushRoute, routeOf, targetOf } from "../target";

describe("where a server path goes", () => {
  it("opens a screen the app has", () => {
    expect(pushRoute("/chat/42")).toBe("/chat/42");
    expect(pushRoute(`${SERVER_URL}/u/alice`)).toBe("/u/alice");
  });

  it("opens what only the web has in the web view, signed in, not the inbox", () => {
    expect(targetOf("/notes/abc?tab=replies")).toEqual({
      type: "web",
      path: "/notes/abc?tab=replies",
    });
    expect(pushRoute("/live/xyz")).toBe(`/web?path=${encodeURIComponent("/live/xyz")}`);
  });

  it("never opens an outside link by itself", () => {
    const target = targetOf("https://example.com/news");
    expect(target).toEqual({ type: "external", url: "https://example.com/news" });
    expect(routeOf(target)).toBeNull();
    expect(pushRoute("https://example.com/news")).toBe(INBOX);
    expect(pushRoute("javascript:alert(1)")).toBe(INBOX);
  });
});
