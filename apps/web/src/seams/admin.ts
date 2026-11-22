import type { LinkProps } from "@tanstack/react-router";
import type { ComponentType } from "react";

/**
 * Staff console sections: the entries of the console's navigation (`/admin`)
 * and of its dashboard.
 *
 * A feature with a review page inside the console (a route `admin.<area>.tsx`)
 * default-exports its entry from `src/features/<area>/admin-section.ts`:
 *
 *   export default {
 *     id: "emoji",
 *     order: 110,
 *     to: "/admin/emoji",
 *     icon: SmilePlus,
 *     needs: "moderator",
 *     label: () => i18n.t("admin:nav.emoji"),
 *   } satisfies AdminSection;
 */
export interface AdminSection {
  id: string;
  /** Position in the navigation, ascending. The console's own sections use 0–60; domains follow from 70. */
  order: number;
  to: NonNullable<LinkProps["to"]>;
  icon: ComponentType<{ className?: string }>;
  /** The access the section's API asks for. */
  needs: "staff" | "moderator" | "admin";
  label(): string;
}

const modules = import.meta.glob<{ default: AdminSection | AdminSection[] }>(
  "../features/*/admin-section.ts",
  { eager: true },
);

/** Every console section, in display order. */
export const ADMIN_SECTIONS: AdminSection[] = Object.values(modules)
  .flatMap((module) => module.default)
  .sort((a, b) => a.order - b.order || a.id.localeCompare(b.id));
