import { type AnyRouter, useRouter } from "@tanstack/react-router";
import { resolveTarget, type Target } from "@tokuly/core/notifications";

export type { Target };

/**
 * Paths the backend sends that live elsewhere in the web app. The test push
 * points at `/settings/notifications`; the notification settings are a page
 * of the inbox here.
 */
const ALIASES: Record<string, string> = {
  "/settings/notifications": "/notifications/settings",
};

/**
 * Reads a notification's `path` (`resolveTarget` of `@tokuly/core/notifications`)
 * for this site, with the web's own aliases. `null` when it leads nowhere.
 */
export function resolveTargetPath(
  path: string,
  origin: string = window.location.origin,
): Target | null {
  const target = resolveTarget(path, origin);
  if (target?.type !== "internal") return target;
  const end = target.href.search(/[?#]/);
  const pathname = end < 0 ? target.href : target.href.slice(0, end);
  const rest = end < 0 ? "" : target.href.slice(end);
  return { type: "internal", href: (ALIASES[pathname] ?? pathname) + rest };
}

/**
 * Goes where a notification points. Returns false when the path leads nowhere,
 * so the caller can stay put.
 */
export function openTargetPath(
  router: AnyRouter,
  path: string,
  options: { replace?: boolean } = {},
): boolean {
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
