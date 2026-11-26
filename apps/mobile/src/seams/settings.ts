import type { LucideIcon } from "lucide-react-native";
import type { ComponentType } from "react";
import cards from "#/generated/seams/settings-card";
import rows from "#/generated/seams/settings-rows";

/**
 * Settings: what a feature adds to the settings screen.
 *
 * A row opens something (a screen, the web); a feature default-exports its
 * rows from `src/features/<area>/settings-rows.ts`:
 *
 *   export default [
 *     { id: "accounts", order: 10, icon: Users, label: () => i18n.t("mobile:settings.accounts"),
 *       onPress: () => router.push("/accounts") },
 *   ] satisfies SettingsRow[];
 *
 * A card is a control that lives on the screen itself (push on this device):
 * `src/features/<area>/settings-card.tsx` default-exports
 *
 *   export default { id: "push", order: 10, Component: PushCard } satisfies SettingsCard;
 */
export interface SettingsRow {
  /** Unique among the rows; the row's test ID is `settings-<id>`. */
  id: string;
  /** Position in the list, ascending: accounts 10, the web's settings 20, licences 30. */
  order: number;
  icon: LucideIcon;
  label(): string;
  /** One line under the label: what is inside. */
  hint?(): string;
  onPress(): void;
}

export interface SettingsCard {
  id: string;
  /** Position under the rows, ascending. */
  order: number;
  Component: ComponentType;
}

const byOrder = (a: { id: string; order: number }, b: { id: string; order: number }) =>
  a.order - b.order || a.id.localeCompare(b.id);

/** Every row, in display order. */
export const SETTINGS_ROWS: SettingsRow[] = rows.flat().sort(byOrder);

/** Every card, in display order. */
export const SETTINGS_CARDS: SettingsCard[] = [...cards].sort(byOrder);
