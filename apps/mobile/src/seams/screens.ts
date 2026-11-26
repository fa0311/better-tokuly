import type { Stack } from "expo-router";
import type { ComponentProps } from "react";
import screens from "#/generated/seams/screens";

/**
 * Screens pushed over the tabs: how each one is presented (its title, a
 * modal, no header).
 *
 * Routes stay files (`app/(app)/chat/[id].tsx`, thin: they render a screen of
 * the feature). A feature whose routes need more than the default header
 * default-exports their options from `src/features/<area>/screens.ts`:
 *
 *   export default [
 *     { name: "chat/[id]", options: () => ({ headerShown: false }) },
 *     { name: "new-chat", options: () => ({ title: i18n.t("mobile:chat.newChat"), presentation: "modal" }) },
 *   ] satisfies ScreenEntry[];
 */
export interface ScreenEntry {
  /** The route's path under `app/(app)/`, without the extension. */
  name: string;
  /** Read on every render of the signed-in layout, so titles follow the language. */
  options(): ScreenOptions;
}

export type ScreenOptions = Exclude<
  ComponentProps<typeof Stack.Screen>["options"],
  undefined | ((...args: never[]) => unknown)
>;

/** Every route with options of its own. */
export const SCREENS: ScreenEntry[] = screens.flat();
