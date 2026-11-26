import { describe, expect, it } from "vitest";
import { contentSecurityPolicy, deploymentFrom, permissionsPolicy } from "./security-headers";

function directive(policy: string, name: string): string[] {
  const found = policy.split("; ").find((part) => part.startsWith(`${name} `));
  return found ? found.split(" ").slice(1) : [];
}

describe("deploymentFrom", () => {
  it("allows the origins the backend hands out", () => {
    const deployment = deploymentFrom({
      TOKULY_PUBLIC_URL: "https://tokuly.example",
      TOKULY_RT_PUBLIC_WT_URL: "https://rt.tokuly.example:4433/rt",
      TOKULY_LIVEKIT_PUBLIC_URL: "wss://lk.tokuly.example",
      TOKULY_LIVE_WHIP_URL: "https://live.tokuly.example:8889",
      TOKULY_LIVE_WHEP_URL: "https://live.tokuly.example:8889",
      TOKULY_LIVE_HLS_URL: "https://live.tokuly.example:8888",
      TOKULY_LIVE_PLAYBACK_URL: "https://live.tokuly.example:9996",
      TOKULY_S3_PUBLIC_ENDPOINT: "https://s3.tokuly.example",
      TOKULY_MEDIA_URL: "https://cdn.tokuly.example/media",
    });
    expect(deployment.connect).toEqual([
      "https://rt.tokuly.example:4433",
      "wss://lk.tokuly.example",
      "https://lk.tokuly.example",
      "https://live.tokuly.example:8889",
      "https://live.tokuly.example:8888",
      "https://live.tokuly.example:9996",
      "https://s3.tokuly.example",
    ]);
    expect(deployment.media).toEqual([
      "https://live.tokuly.example:8888",
      "https://live.tokuly.example:9996",
    ]);
    expect(deployment.files).toEqual(["https://cdn.tokuly.example"]);
  });

  it("falls back to the backend's defaults, and leaves out what is off", () => {
    const deployment = deploymentFrom({
      TOKULY_PUBLIC_URL: "https://tokuly.example",
      TOKULY_RT_PUBLIC_WT_URL: "",
      TOKULY_LIVEKIT_PUBLIC_URL: "",
    });
    expect(deployment.connect).toEqual([
      "https://tokuly.example:4433",
      "http://localhost:8889",
      "http://localhost:8888",
      "http://localhost:9996",
      "http://localhost:9000",
    ]);
  });
});

describe("contentSecurityPolicy", () => {
  const deployment = { connect: ["https://rt.tokuly.example:4433"], media: [], files: [] };

  it("runs scripts from this origin and with the nonce only", () => {
    const policy = contentSecurityPolicy(deployment, "abc");
    expect(directive(policy, "script-src")).toEqual(["'self'", "'nonce-abc'"]);
    expect(directive(policy, "object-src")).toEqual(["'none'"]);
    expect(directive(policy, "base-uri")).toEqual(["'self'"]);
    expect(directive(policy, "connect-src")).toEqual(["'self'", "https://rt.tokuly.example:4433"]);
  });

  it("lets only embeddable pages be framed", () => {
    expect(directive(contentSecurityPolicy(deployment, "n"), "frame-ancestors")).toEqual([
      "'none'",
    ]);
    expect(
      directive(contentSecurityPolicy(deployment, "n", { embeddable: true }), "frame-ancestors"),
    ).toEqual(["*"]);
  });
});

describe("permissionsPolicy", () => {
  it("allows what the app uses to itself, and the players what they need", () => {
    const policy = permissionsPolicy();
    expect(policy).toContain("geolocation=(self)");
    expect(policy).toContain("camera=(self)");
    expect(policy).toContain(
      'fullscreen=(self "https://www.youtube-nocookie.com" "https://open.spotify.com")',
    );
    expect(policy).toContain("payment=()");
  });

  it("turns everything off on embeddable pages", () => {
    const policy = permissionsPolicy({ embeddable: true });
    expect(policy).toContain("geolocation=()");
    expect(policy).not.toContain("self");
  });
});
