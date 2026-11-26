import { expect, test } from "./support";

/** The sources of `name` in a Content-Security-Policy. */
function directive(policy: string, name: string): string[] {
  const found = policy.split(/;\s*/).find((part) => part.startsWith(`${name} `));
  return found ? found.split(" ").slice(1) : [];
}

test.describe("security headers", () => {
  for (const path of ["/", "/login", "/me", "/docs/api", "/oauth/authorize"]) {
    test(`${path} has a strict Content-Security-Policy and a Permissions-Policy`, async ({
      request,
    }) => {
      const response = await request.get(path);
      const headers = response.headers();
      const policy = headers["content-security-policy"] ?? "";
      const scripts = directive(policy, "script-src");
      expect(scripts.filter((source) => !source.startsWith("'nonce-"))).toEqual(["'self'"]);
      expect(scripts.some((source) => source.startsWith("'nonce-"))).toBe(true);
      expect(directive(policy, "object-src")).toEqual(["'none'"]);
      expect(directive(policy, "base-uri")).toEqual(["'self'"]);
      expect(directive(policy, "frame-ancestors")).toEqual(["'none'"]);
      expect(headers["x-frame-options"]).toBe("DENY");
      expect(headers["permissions-policy"]).toContain("geolocation=(self)");
      expect(headers["permissions-policy"]).toContain("payment=()");
    });
  }

  test("every script of a page carries the nonce of its response", async ({ request }) => {
    const response = await request.get("/login");
    const policy = response.headers()["content-security-policy"] ?? "";
    const nonce = /'nonce-([^']+)'/.exec(policy)?.[1];
    expect(nonce).toBeTruthy();
    // No shared cache may hand the same page, and so the same nonce, to someone else.
    expect(response.headers()["cache-control"]).toMatch(/private|no-store/);
    const html = await response.text();
    const inline = [...html.matchAll(/<script(?![^>]*\ssrc=)([^>]*)>/g)].map((match) => match[1]);
    expect(inline.length).toBeGreaterThan(0);
    for (const attributes of inline) expect(attributes).toContain(`nonce="${nonce}"`);
    // A new one for every response.
    const again = (await request.get("/login")).headers()["content-security-policy"];
    expect(again).not.toContain(`'nonce-${nonce}'`);
  });

  test("an OBS widget may be framed and gets no powerful features", async ({ request }) => {
    const response = await request.get("/overlay/not-a-real-token");
    const headers = response.headers();
    const policy = headers["content-security-policy"] ?? "";
    expect(directive(policy, "frame-ancestors")).toEqual(["*"]);
    expect(directive(policy, "object-src")).toEqual(["'none'"]);
    expect(headers["x-frame-options"]).toBeUndefined();
    expect(headers["permissions-policy"]).toContain("camera=()");
    expect(headers["permissions-policy"]).not.toContain("self");
  });
});
