import { API_PATH } from "#/lib/api";

/** Where this instance serves the pictures of link previews (see the media domain's `image_proxy`). */
const PROXY_PATH = `${API_PATH}/media/link-image/`;

/**
 * The address to load a link preview's picture (`imageUrl`, `faviconUrl`) from, or
 * `undefined` when there is none to show.
 *
 * The server hands out pictures as URLs of this instance, which fetches them on the
 * viewer's behalf: loading them from the other site would tell it who looked. Anything
 * else (a preview stored before the proxy existed, or one from elsewhere) is not loaded
 * at all. Proxy URLs are loaded from the app's own origin, like every API call.
 *
 * A picture that is one of this instance's own public files (`/media/…` on the app's
 * origin) is not proxied by the server, so that it disappears the moment the file is
 * deleted or taken down; it is loaded as it is.
 */
export function linkPreviewImage(
  url: string | undefined,
  origin = window.location.origin,
): string | undefined {
  if (!url) return undefined;
  let parsed: URL;
  try {
    parsed = new URL(url, origin);
  } catch {
    return undefined;
  }
  if (parsed.search || parsed.hash) return undefined;
  if (parsed.pathname.startsWith(PROXY_PATH)) return `${origin}${parsed.pathname}`;
  if (parsed.origin === origin && parsed.pathname.startsWith("/media/")) return parsed.href;
  return undefined;
}
