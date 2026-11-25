import { useNotificationsSync } from "./sync";

/**
 * Runs while someone is signed in, on every screen: the inbox and the icon
 * badge stay live, push follows the account's choice, taps on pushes open
 * their screens.
 */
export function NotificationsBackground() {
  useNotificationsSync();
  return null;
}
