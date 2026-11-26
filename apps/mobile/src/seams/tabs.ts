import type { LucideIcon } from "lucide-react-native";
import tabs from "#/generated/seams/tabs";

/**
 * Tabs: the destinations one tap away, at the bottom of every signed-in screen.
 *
 * A feature with a tab default-exports it from `src/features/<area>/tabs.tsx`
 * and adds the route file `app/(app)/(tabs)/<name>.tsx`:
 *
 *   export default [
 *     { name: "chats", order: 20, title: () => i18n.t("mobile:tabs.chat"),
 *       icon: MessageCircle, badge: "chat" },
 *   ] satisfies TabEntry[];
 *
 * The bar holds five or six: an area that is not opened many times a day
 * belongs on the hub (`#/seams/hub`).
 */
export interface TabEntry {
  /** The route's file name under `app/(app)/(tabs)/`; the button's test ID is `tab-<name>`. */
  name: string;
  /** Position in the bar, ascending. Leave gaps (10, 20, …). */
  order: number;
  /** The tab's name in the current language. */
  title(): string;
  icon: LucideIcon;
  /** The screen fills the whole height itself (the map, a feed): no header above it. */
  noHeader?: boolean;
  /** Key of the unread counter to show (`#/seams/badges`). */
  badge?: string;
}

/** Every tab, in the bar's order. */
export const TABS: TabEntry[] = tabs
  .flat()
  .sort((a, b) => a.order - b.order || a.name.localeCompare(b.name));
