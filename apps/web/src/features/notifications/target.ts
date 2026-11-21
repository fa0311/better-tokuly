import { type AnyRouter, useRouter } from "@tanstack/react-router";

/** Where a server-sent "path in the app" leads. */
export type Target =
  /** A page of this app: path, search and hash, ready for the router. */
  | { type: "internal"; href: string }
  /** Somewhere else on the web; opened in a new tab. */
  | { type: "external"; url: string };

/**
 * Paths the backend sends that live elsewhere in the web app. The test push
 * points at `/settings/notifications`; the notification settings are a page
 * of the inbox here.
 */
const ALIASES: Record<string, string> = {
  "/settings/notifications": "/notifications/settings",
};

/**
 * Reads a notification's `path` (or any target the server hands out: `/chat/42`,
 * `/u/alice?tab=notes`, a full URL of this site, occasionally an outside link).
 * `null` when there is nowhere to go: an empty path, or a scheme that is not http(s).
 */
export function resolveTargetPath(path: string, origin: string = window.location.origin): Target | null {
  const trimmed = path.trim();
  if (!trimmed) return null;
  let url: URL;
  try {
    url = new URL(trimmed, origin);
  } catch {
    return null;
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") return null;
  if (url.origin !== origin) return { type: "external", url: url.href };
  const pathname = url.pathname.length > 1 ? url.pathname.replace(/\/+$/, "") : url.pathname;
  return { type: "internal", href: (ALIASES[pathname] ?? pathname) + url.search + url.hash };
}

/**
 * Goes where a notification points. Returns false when the path leads nowhere,
 * so the caller can stay put.
 */
export function openTargetPath(router: AnyRouter, path: string, options: { replace?: boolean } = {}): boolean {
  const target = resolveTargetPath(path);
  if (!target) return false;
  if (target.type === "external") window.open(target.url, "_blank", "noopener,noreferrer");
  else void router.navigate({ href: target.href, replace: options.replace });
  return true;
}

/** `openTargetPath` bound to the app's router, for components. */
export function useOpenTargetPath(): (path: string, options?: { replace?: boolean }) => boolean {
  const router = useRouter();
  return (path, options) => openTargetPath(router, path, options);
}
