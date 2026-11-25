import { isDeepLink, routeFor, SIGNED_IN_HOME, SIGNED_OUT_HOME } from "../routing";

describe("routeFor", () => {
  it("waits while the session is restored", () => {
    for (const area of ["root", "app", "auth"] as const)
      expect(routeFor("unknown", area)).toBeNull();
  });

  it("starts a signed-out app on sign-in and a signed-in one on the map", () => {
    expect(routeFor("signedOut", "root")).toBe(SIGNED_OUT_HOME);
    expect(routeFor("signedIn", "root")).toBe(SIGNED_IN_HOME);
  });

  it("keeps the signed-in screens from someone signed out, and sign-in from someone signed in", () => {
    expect(routeFor("signedOut", "app")).toBe(SIGNED_OUT_HOME);
    expect(routeFor("signedIn", "app")).toBeNull();
    expect(routeFor("signedIn", "auth")).toBe(SIGNED_IN_HOME);
    expect(routeFor("signedOut", "auth")).toBeNull();
  });

  it("opens a deep link remembered before sign-in once signed in", () => {
    expect(routeFor("signedIn", "auth", "/invite/abc")).toBe("/invite/abc");
    expect(routeFor("signedIn", "root", "/u/tokuly")).toBe("/u/tokuly");
    expect(routeFor("signedOut", "root", "/invite/abc")).toBe(SIGNED_OUT_HOME);
  });
});

describe("isDeepLink", () => {
  it("remembers links into the app, not its start", () => {
    expect(isDeepLink("/invite/abc")).toBe(true);
    expect(isDeepLink("/")).toBe(false);
    expect(isDeepLink(SIGNED_IN_HOME)).toBe(false);
  });
});
