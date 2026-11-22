import { useEffect } from "react";
import { useBadges } from "#/stores/badges";
import { useSessionStore } from "#/stores/session";

type Badging = Navigator & {
  setAppBadge?(count: number): Promise<void>;
  clearAppBadge?(): Promise<void>;
};

/** Puts `count` on the installed app's icon (Badging API); nothing happens where there is no such thing. */
function showOnIcon(count: number): void {
  const badging = navigator as Badging;
  void (count > 0 ? badging.setAppBadge?.(count) : badging.clearAppBadge?.())?.catch(() => {});
}

/**
 * The installed app's icon carries the number of unread notifications: the
 * same number the server sends with a push, which the service worker shows
 * while the app is closed. Chat messages and friend requests arrive as
 * notifications, so they are in it. This is the only place a page sets it.
 */
export function useAppBadge(): void {
  const total = useBadges((badges) => badges.notifications ?? 0);

  useEffect(() => {
    showOnIcon(total);
  }, [total]);

  useEffect(
    () => () => {
      // Signed out: the icon says nothing about the account that left.
      // (Checked, because development double-mounts effects while still signed in.)
      if (useSessionStore.getState().state.status !== "signedIn") showOnIcon(0);
    },
    [],
  );
}
