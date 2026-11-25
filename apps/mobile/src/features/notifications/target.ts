import { resolveTarget } from "@tokuly/core/notifications";
import { SERVER_URL } from "#/lib/config";

/** Where a path the server hands out goes in this app. */
export type AppTarget =
  /** A screen of the app, ready for the router. */
  | { type: "screen"; href: string }
  /** Not in the app yet: opened in the in-app browser. */
  | { type: "web"; url: string };

export const INBOX = "/inbox";

/** The server's paths that have a screen here, and the screen. */
const SCREENS: [RegExp, (match: RegExpMatchArray) => string][] = [
  [/^\/chat\/([^/]+)$/, (m) => `/chat/${m[1]}`],
  [/^\/u\/([^/]+)$/, (m) => `/u/${m[1]}`],
  [/^\/invite\/([^/]+)$/, (m) => `/invite/${m[1]}`],
  [/^\/friends(\/.*)?$/, () => "/friends"],
  [/^\/map$/, () => "/map"],
  [/^\/notifications$/, () => INBOX],
];

/**
 * Reads a notification's `path` (`resolveTarget` of `@tokuly/core/notifications`
 * against this build's server). `null`: it leads nowhere.
 */
export function targetOf(path: string): AppTarget | null {
  const target = resolveTarget(path, SERVER_URL);
  if (!target) return null;
  if (target.type === "external") return { type: "web", url: target.url };
  const pathname = target.href.split(/[?#]/)[0] ?? "";
  for (const [pattern, screen] of SCREENS) {
    const match = pathname.match(pattern);
    if (match) return { type: "screen", href: screen(match) };
  }
  return { type: "web", url: `${SERVER_URL}${target.href}` };
}

/**
 * Where a tap on a push goes: its screen, else the inbox (whose entry can
 * open the web). A push never opens a browser by itself.
 */
export function pushRoute(path: string): string {
  const target = targetOf(path);
  return target?.type === "screen" ? target.href : INBOX;
}
