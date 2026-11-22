import type { common, identity } from "@tokuly/api";
import type { ComponentType } from "react";

/**
 * Profile sections: what a feature shows on a person's profile (`/u/<handle>`).
 *
 * A feature default-exports its sections from `src/features/<area>/profile.tsx`
 * and the profile screen places them by `slot` and `order`:
 *
 *   export default [
 *     { id: "plans", slot: "card", order: 10, Component: ({ profile, user, isSelf }) => (
 *         <ProfilePlans cards={profile.cards} user={user} isSelf={isSelf} />
 *       ) },
 *   ] satisfies ProfileSection[];
 *
 * A component that has nothing to show for this person renders nothing. One
 * that does brings its own distance to what is above it (`mt-5`), except in
 * the `action` and `stat` slots, whose rows space their items.
 */
export interface ProfileSection {
  /** Unique across the app: the area's name, or `<area>-<what>`. */
  id: string;
  slot: ProfileSlot;
  /** Position inside the slot, ascending. Leave gaps (10, 20, …). */
  order: number;
  Component: ComponentType<ProfileSectionProps>;
}

/**
 * Where a section goes, top to bottom:
 * - `status`: right under the bio — what the person is doing now (studying, in class);
 * - `action`: a button in the row of actions (message, add friend, …);
 * - `card`: cards between the actions and the numbers (free now, countdowns, highlights);
 * - `stat`: one or more cells of the numbers card (`<ProfileStat>`);
 * - `content`: the long lists at the bottom (answers, notes). Not shown for a blocked person.
 */
export type ProfileSlot = "status" | "action" | "card" | "stat" | "content";

export interface ProfileSectionProps {
  profile: identity.Profile;
  /** `profile.user`, always present. */
  user: common.UserSummary;
  /** The profile is the viewer's own. */
  isSelf: boolean;
  isFriend: boolean;
}

const modules = import.meta.glob<{ default: ProfileSection[] }>("../features/*/profile.tsx", {
  eager: true,
});

const SECTIONS: ProfileSection[] = Object.values(modules)
  .flatMap((module) => module.default)
  .sort((a, b) => a.order - b.order || a.id.localeCompare(b.id));

/** The sections of one slot, in order. */
export function profileSections(slot: ProfileSlot): ProfileSection[] {
  return SECTIONS.filter((section) => section.slot === slot);
}

/** Every section of a slot, rendered for one profile. */
export function ProfileSlotView({ slot, ...props }: ProfileSectionProps & { slot: ProfileSlot }) {
  return profileSections(slot).map((section) => <section.Component key={section.id} {...props} />);
}

/** One cell of the profile's numbers card. */
export function ProfileStat({ value, label }: { value: number | string; label: string }) {
  return (
    <div>
      <div className="text-xl font-black">
        {typeof value === "number" ? value.toLocaleString() : value}
      </div>
      <div className="text-xs font-bold text-ink-faint">{label}</div>
    </div>
  );
}
