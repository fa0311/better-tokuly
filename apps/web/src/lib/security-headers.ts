/**
 * The Content-Security-Policy and Permissions-Policy of the app's pages
 * (applied in `src/start.ts`).
 *
 * Scripts run only from this origin or with the nonce of the response, so
 * text that slips into a page as markup cannot run. Connections go to this
 * origin and to the parts of the deployment browsers reach directly (the
 * realtime gateway's WebTransport, LiveKit, the media server, uploads to the
 * object store, a CDN in front of the media); those addresses are read from the same `TOKULY_*` variables
 * the backend hands them out from, so the two always agree. The only other
 * sites are the players of the link previews (YouTube, Spotify).
 */

/** The players a link preview opens in place (`frame-src`). */
export const EMBED_ORIGINS = ["https://www.youtube-nocookie.com", "https://open.spotify.com"];

/** The other origins browsers reach while using the app. */
export interface Deployment {
  /** `connect-src`: fetches, WebSockets, WebTransport. */
  connect: string[];
  /** `media-src`: what `<video>` plays from a URL (HLS on Safari, recordings). */
  media: string[];
  /** Where public media is served from when it is not this origin's `/media` (a CDN). */
  files: string[];
  /** Development: a mini app may be served over `http` from a loopback address. */
  loopbackMiniApps: boolean;
}

type Env = Record<string, string | undefined>;

/**
 * The deployment's addresses, from the backend's variables. An unset variable
 * means the backend's default (see `tokuly.example.toml`); an empty one means
 * the feature is off.
 */
export function deploymentFrom(env: Env): Deployment {
  const value = (name: string, fallback = "") => env[name] ?? fallback;
  const livekit = value("TOKULY_LIVEKIT_PUBLIC_URL");
  const hls = value("TOKULY_LIVE_HLS_URL", "http://localhost:8888");
  const playback = value("TOKULY_LIVE_PLAYBACK_URL", "http://localhost:9996");
  const files = origins([value("TOKULY_MEDIA_URL")]);
  const connect = [
    ...realtimeUrls(env),
    // LiveKit's client checks the server over HTTP(S) before opening the socket.
    livekit,
    livekit.replace(/^ws/, "http"),
    value("TOKULY_LIVE_WHIP_URL", "http://localhost:8889"),
    value("TOKULY_LIVE_WHEP_URL", "http://localhost:8889"),
    hls,
    playback,
    value("TOKULY_S3_PUBLIC_ENDPOINT", value("TOKULY_S3_ENDPOINT", "http://localhost:9000")),
  ];
  return {
    connect: origins(connect),
    media: origins([hls, playback]),
    files,
    loopbackMiniApps: env.TOKULY_ENV !== "production",
  };
}

/**
 * The one other origin the mini-app page (`/miniapps/<id>?o=<origin>`) may
 * frame: the approved origin it was opened for, as the address names it.
 * The page frames a mini app only after the API confirmed that origin (and
 * goes back to the right address when it differs), so a hand-made address
 * allows a frame nothing ever loads. `undefined` for any other page and for
 * anything that is not a bare `https` origin (`http` on a loopback address
 * in development).
 */
export function miniAppFrameOrigin(deployment: Deployment, url: URL): string | undefined {
  if (!/^\/miniapps\/[^/]+\/?$/.test(url.pathname)) return undefined;
  const named = url.searchParams.get("o") ?? "";
  const origin = parsed(named)?.origin;
  if (!origin || origin !== named) return undefined;
  const { protocol, hostname } = new URL(origin);
  if (protocol === "https:") return origin;
  const loopback = hostname === "127.0.0.1" || hostname === "[::1]";
  return protocol === "http:" && loopback && deployment.loopbackMiniApps ? origin : undefined;
}

/**
 * Where the realtime gateway tells clients to connect: the rules of the
 * backend's `realtime/src/endpoints.rs`. WebTransport is the gateway's own
 * port on the public host. The WebSocket is `/rt/ws` of the public origin in
 * production (named anyway: older browsers do not count `ws:` as `'self'`)
 * and the gateway's port in development.
 */
function realtimeUrls(env: Env): string[] {
  const configured = (name: string) => env[name]?.trim() || undefined;
  const port = (name: string, fallback: string) => (env[name] ?? fallback).split(":").pop();
  const publicUrl = parsed(env.TOKULY_PUBLIC_URL ?? "") ?? new URL("http://localhost:8080");
  const wtPort = port("TOKULY_RT_WT_LISTEN", "0.0.0.0:4433");
  const webTransport = wtPort
    ? (configured("TOKULY_RT_PUBLIC_WT_URL") ?? `https://${publicUrl.hostname}:${wtPort}`)
    : "";
  const webSocket =
    configured("TOKULY_RT_PUBLIC_WS_URL") ??
    (env.TOKULY_ENV === "production"
      ? publicUrl.origin.replace(/^http/, "ws")
      : `ws://${publicUrl.hostname}:${port("TOKULY_RT_WS_LISTEN", "0.0.0.0:8082")}`);
  return [webTransport, webSocket];
}

/**
 * The `Content-Security-Policy` of a page. `embeddable` pages may be framed by
 * any site; `miniApp` is the one mini-app origin the page may frame.
 */
export function contentSecurityPolicy(
  deployment: Deployment,
  nonce: string,
  { embeddable = false, miniApp }: { embeddable?: boolean; miniApp?: string } = {},
): string {
  const directives: Record<string, string[]> = {
    "default-src": ["'self'"],
    "script-src": ["'self'", `'nonce-${nonce}'`],
    // React renders `style` attributes and libraries add `<style>` elements.
    "style-src": ["'self'", "'unsafe-inline'"],
    // QR codes and pictures chosen for upload are `data:` and `blob:` URLs.
    "img-src": ["'self'", "data:", "blob:", ...deployment.files],
    // The build inlines small font files as `data:` URLs.
    "font-src": ["'self'", "data:"],
    // HLS plays from `blob:` URLs of the media source.
    "media-src": ["'self'", "blob:", ...deployment.media, ...deployment.files],
    "connect-src": ["'self'", ...deployment.connect, ...deployment.files],
    "worker-src": ["'self'", "blob:"],
    "frame-src": miniApp ? [...EMBED_ORIGINS, miniApp] : EMBED_ORIGINS,
    "object-src": ["'none'"],
    "base-uri": ["'self'"],
    "form-action": ["'self'"],
    // A page with buttons inside a hostile frame can be clicked through a decoy.
    "frame-ancestors": embeddable ? ["*"] : ["'none'"],
  };
  return Object.entries(directives)
    .map(([name, sources]) => `${name} ${sources.join(" ")}`)
    .join("; ");
}

/** What the app uses, and the embedded players with it where they need it. */
const USED: Record<string, string[]> = {
  geolocation: [],
  camera: [],
  microphone: [],
  "display-capture": [],
  fullscreen: EMBED_ORIGINS,
  "picture-in-picture": EMBED_ORIGINS,
  autoplay: EMBED_ORIGINS,
  "encrypted-media": EMBED_ORIGINS,
  "clipboard-write": EMBED_ORIGINS,
  "publickey-credentials-create": [],
  "publickey-credentials-get": [],
  // A bump (two phones knocked together) is read from the motion sensors.
  accelerometer: [],
  gyroscope: [],
};

/**
 * Powerful features the app never uses: off for the page and anything it frames.
 * Only names Chrome knows (it reports any other): sharing (`web-share`) is
 * left at its default, this origin.
 */
const UNUSED = [
  "browsing-topics",
  "compute-pressure",
  "gamepad",
  "hid",
  "identity-credentials-get",
  "idle-detection",
  "local-fonts",
  "magnetometer",
  "midi",
  "otp-credentials",
  "payment",
  "screen-wake-lock",
  "serial",
  "storage-access",
  "usb",
  "window-management",
  "xr-spatial-tracking",
];

/**
 * The `Permissions-Policy` of a page. An `embeddable` page (an OBS widget)
 * only shows things: every feature is off.
 */
export function permissionsPolicy({ embeddable = false } = {}): string {
  const used = Object.entries(USED).map(([feature, others]) =>
    embeddable ? `${feature}=()` : `${feature}=(${["self", ...others.map(quote)].join(" ")})`,
  );
  return [...used, ...UNUSED.map((feature) => `${feature}=()`)].join(", ");
}

function quote(origin: string): string {
  return `"${origin}"`;
}

function parsed(url: string): URL | undefined {
  try {
    const parsed = new URL(url);
    return parsed.origin === "null" ? undefined : parsed;
  } catch {
    return undefined;
  }
}

function origins(urls: string[]): string[] {
  return [
    ...new Set(urls.map((url) => parsed(url)?.origin).filter((value) => value !== undefined)),
  ];
}
