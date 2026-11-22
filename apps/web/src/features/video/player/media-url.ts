/**
 * The address to load a media file from when script reads it (the HLS
 * playlists and segments, subtitle and seek-preview tracks, frames drawn to a
 * canvas) rather than an `<img>` or `<video>` tag.
 *
 * In production the page and `/media` share one origin, so this is the URL as
 * given. A development server, though, may talk to an API whose public URL is
 * another dev server: script would then be refused across origins. There the
 * file is fetched through this server's own `/media` route instead, which
 * forwards to the same object store.
 */
export function scriptUrl(url: string): string {
  if (!import.meta.env.DEV || !url || typeof window === "undefined") return url;
  try {
    const parsed = new URL(url, window.location.origin);
    const local = /^(localhost|127\.0\.0\.1)$/.test(parsed.hostname);
    return local && parsed.origin !== window.location.origin && parsed.pathname.startsWith("/media/") ? parsed.pathname + parsed.search : url;
  } catch {
    return url;
  }
}
