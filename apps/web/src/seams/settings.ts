import type { LinkProps } from "@tanstack/react-router";
import type { ComponentType } from "react";
import { namespacesOf } from "#/lib/i18n";

/**
 * Settings sections: the rows of `/settings` (and of the list next to every
 * settings page on wide screens).
 *
 * A feature with a settings page of its own default-exports a row from
 * `src/features/<area>/settings-section.ts`:
 *
 *   export default {
 *     id: "comments",
 *     order: 45,
 *     to: "/settings/comments",
 *     icon: MessageSquareText,
 *     tint: "bg-sky text-[#17122b]",
 *     title: "comments:settingsRow.title",
 *     hint: "comments:settingsRow.hint",
 *   } satisfies SettingsSection;
 *
 * Give the page itself a route under `/settings/…` when it should open next
 * to the list on wide screens.
 */
export interface SettingsSection {
  id: string;
  /** Position in the list, ascending: profile 10, account 20, security 30, privacy 40, appearance 60, connections 80, data 90. */
  order: number;
  to: NonNullable<LinkProps["to"]>;
  icon: ComponentType<{ className?: string }>;
  /** Background and text colour of the icon bubble. */
  tint: string;
  title: string;
  /** One line under the title: what is inside. */
  hint: string;
  /** A hook returning a short note when the section wants attention (shown instead of the hint, with a dot). */
  useAttention?(): string | undefined;
}

const modules = import.meta.glob<{ default: SettingsSection | SettingsSection[] }>(
  "../features/*/settings-section.ts",
  { eager: true },
);

/** Every settings row, in display order. */
export const SETTINGS_SECTIONS: SettingsSection[] = Object.values(modules)
  .flatMap((module) => module.default)
  .sort((a, b) => a.order - b.order || a.id.localeCompare(b.id));

/** What `useTranslation` loads before the rows' titles show. */
export const SETTINGS_NAMESPACES = namespacesOf(
  SETTINGS_SECTIONS.flatMap((section) => [section.title, section.hint]),
);
