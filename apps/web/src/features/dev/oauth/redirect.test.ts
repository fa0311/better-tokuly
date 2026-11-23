import { describe, expect, it } from "vitest";
import { authorizationParams, carriedQuery, carryQuery, redirectTarget } from "./redirect";

describe("redirectTarget", () => {
  it("follows the kinds of redirect URI an application can register", () => {
    expect(redirectTarget("https://app.example/cb?code=1&state=2")).toBe(
      "https://app.example/cb?code=1&state=2",
    );
    expect(redirectTarget("http://127.0.0.1:8123/callback?code=1")).toBe(
      "http://127.0.0.1:8123/callback?code=1",
    );
    expect(redirectTarget("com.example.app:/callback?code=1")).toBe(
      "com.example.app:/callback?code=1",
    );
  });

  it("refuses addresses that would run or stay in this page", () => {
    for (const url of [
      "javascript:alert(1)",
      "JavaScript:alert(1)",
      "data:text/html,<script>1</script>",
      "blob:https://tokuly.example/1",
      "/settings",
      "//evil.example",
      "",
    ])
      expect(redirectTarget(url), url).toBeUndefined();
  });
});

describe("the authorization request in the address", () => {
  const SEARCH =
    "?response_type=code&client_id=abc&redirect_uri=https%3A%2F%2Fapp.example%2Fcb&scope=notes%3Aread+geo%3Aread&state=1e3&code_challenge=xyz&code_challenge_method=S256&approve=1";

  it("reads the OAuth parameters as they were written and nothing else", () => {
    expect(authorizationParams(new URLSearchParams(SEARCH))).toEqual({
      clientId: "abc",
      redirectUri: "https://app.example/cb",
      responseType: "code",
      scope: "notes:read geo:read",
      state: "1e3",
      codeChallenge: "xyz",
      codeChallengeMethod: "S256",
    });
  });

  it("carries a query string through sign-in unchanged", () => {
    const carried = carryQuery(SEARCH);
    expect(carried).toMatch(/^q\.[A-Za-z0-9_-]+$/);
    expect(carriedQuery(`?request=${carried}`).toString()).toBe(
      new URLSearchParams(SEARCH).toString(),
    );
    expect(carriedQuery("?state=%E3%81%82").get("state")).toBe("あ");
    expect(carriedQuery(`?request=${carryQuery("?state=あ&x=1")}`).get("state")).toBe("あ");
  });

  it("falls back to the page's own query when nothing valid is carried", () => {
    expect(carriedQuery("?request=q.%%%&state=s").get("state")).toBe("s");
    expect(carriedQuery("?request=other&state=s").get("state")).toBe("s");
  });
});
