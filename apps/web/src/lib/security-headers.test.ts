import { describe, expect, it } from "vitest";
import { contentSecurityPolicy, deploymentFrom, permissionsPolicy } from "./security-headers";

function directive(policy: string, name: string): string[] {
  const found = policy.split("; ").find((part) => part.startsWith(`${name} `));
  return found ? found.split(" ").slice(1) : [];
}

describe("deploymentFrom", () => {
  it("allows the origins the backend hands out", () => {
    const deployment = deploymentFrom({
      TOKULY_ENV: "production",
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
      "wss://tokuly.example",
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
      "ws://tokuly.example:8082",
      "http://localhost:8889",
      "http://localhost:8888",
      "http://localhost:9996",
      "http://localhost:9000",
    ]);
  });
});

describe("deploymentFrom, the realtime gateway", () => {
  const realtime = (env: Record<string, string>) => deploymentFrom(env).connect.slice(0, 2);

  it("is reached through the edge in production", () => {
    expect(
      realtime({ TOKULY_ENV: "production", TOKULY_PUBLIC_URL: "https://tokuly.example" }),
    ).toEqual(["https://tokuly.example:4433", "wss://tokuly.example"]);
    expect(realtime({ TOKULY_ENV: "production", TOKULY_PUBLIC_URL: "http://localhost" })).toEqual([
      "https://localhost:4433",
      "ws://localhost",
    ]);
  });

  it("is reached on its own ports in development", () => {
    expect(realtime({})).toEqual(["https://localhost:4433", "ws://localhost:8082"]);
    expect(realtime({ TOKULY_RT_WS_LISTEN: "0.0.0.0:58082" })[1]).toBe("ws://localhost:58082");
  });

  it("uses the configured addresses, and none for WebTransport when it is off", () => {
    const env = {
      TOKULY_ENV: "production",
      TOKULY_PUBLIC_URL: "https://tokuly.example",
      TOKULY_RT_PUBLIC_WT_URL: "https://rt.tokuly.example/rt",
      TOKULY_RT_PUBLIC_WS_URL: " wss://rt.tokuly.example/rt/ws ",
    };
    expect(realtime(env)).toEqual(["https://rt.tokuly.example", "wss://rt.tokuly.example"]);
    expect(realtime({ ...env, TOKULY_RT_WT_LISTEN: "" })[0]).toBe("wss://rt.tokuly.example");
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
