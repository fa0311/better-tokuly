import { resolveTarget } from "@tokuly/core/notifications";
import { SERVER_URL } from "#/lib/config";
import { screenFor } from "#/seams/links";

/** Where a path the server hands out goes in this app. */
export type AppTarget =
  /** A screen of the app, ready for the router. */
  | { type: "screen"; href: string }
  /** Not in the app yet: opened in the in-app browser. */
  | { type: "web"; url: string };

export const INBOX = "/inbox";

/**
 * Reads a notification's `path` (`resolveTarget` of `@tokuly/core/notifications`
 * against this build's server). The features say which of the server's paths
 * have a screen here (`#/seams/links`). `null`: it leads nowhere.
 */
export function targetOf(path: string): AppTarget | null {
  const target = resolveTarget(path, SERVER_URL);
  if (!target) return null;
  if (target.type === "external") return { type: "web", url: target.url };
  const screen = screenFor(target.href);
  if (screen) return { type: "screen", href: screen };
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
