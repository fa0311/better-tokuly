import { useRouter } from "@tanstack/react-router";

/**
 * Opens a "path in the app" the server handed out (a subject's page in a
 * bookmark, a report or an activity entry). Paths on this origin go through
 * the router; anything else opens in a new tab; an empty path does nothing.
 */
export function useOpenPath(): (path: string) => void {
  const router = useRouter();
  return (path) => {
    const trimmed = path.trim();
    if (!trimmed) return;
    let url: URL;
    try {
      url = new URL(trimmed, window.location.origin);
    } catch {
      return;
    }
    if (url.protocol !== "http:" && url.protocol !== "https:") return;
    if (url.origin !== window.location.origin) window.open(url.href, "_blank", "noopener,noreferrer");
    else void router.navigate({ href: url.pathname + url.search + url.hash });
  };
}
