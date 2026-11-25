import type { SessionState } from "@tokuly/api";
import { create } from "zustand";

/** Where a signed-in start lands, and where everyone else does. */
export const SIGNED_IN_HOME = "/map";
export const SIGNED_OUT_HOME = "/welcome";

/** Which part of the app a route belongs to: `/` itself, the signed-in screens, or sign-in. */
export type RouteArea = "root" | "app" | "auth";

/**
 * Where to send someone who opened `path` in `area`, or `null` to show it.
 * Nothing moves while the session is still being restored (the splash
 * screen is up). A link into the signed-in app opened while signed out
 * goes to sign-in and is remembered (`link`); once signed in, it is opened
 * instead of the map.
 */
export function routeFor(
  status: SessionState["status"],
  area: RouteArea,
  link: string | null = null,
): string | null {
  if (status === "unknown") return null;
  const signedIn = status === "signedIn";
  switch (area) {
    case "root":
      return signedIn ? (link ?? SIGNED_IN_HOME) : SIGNED_OUT_HOME;
    case "app":
      return signedIn ? null : SIGNED_OUT_HOME;
    case "auth":
      return signedIn ? (link ?? SIGNED_IN_HOME) : null;
  }
}

/** Whether a path is worth coming back to after sign-in (not the bare start of the app). */
export function isDeepLink(path: string): boolean {
  return path !== "/" && path !== "" && path !== SIGNED_IN_HOME;
}

interface PendingLink {
  /** A link into the signed-in app that waits for sign-in. */
  link: string | null;
  remember(path: string): void;
  clear(): void;
}

export const usePendingLink = create<PendingLink>((set) => ({
  link: null,
  remember: (path) => set({ link: isDeepLink(path) ? path : null }),
  clear: () => set({ link: null }),
}));
