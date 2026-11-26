import type { common, identity } from "@tokuly/api";
import type { ComponentType } from "react";
import profile from "#/generated/seams/profile";

/**
 * Profile sections: what a feature shows on a person's profile.
 *
 * A feature default-exports its sections from `src/features/<area>/profile.tsx`
 * and the profile screen places them by `slot` and `order`:
 *
 *   export default [
 *     { id: "stories", slot: "content", order: 10, Component: ({ user }) => <ProfileStories user={user} /> },
 *   ] satisfies ProfileSection[];
 *
 * A component that has nothing to show for this person renders nothing.
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
 * - `action`: a button under who the person is to the viewer (message, add friend);
 * - `content`: cards and lists under the numbers (stories, highlights, notes).
 */
export type ProfileSlot = "action" | "content";

export interface ProfileSectionProps {
  profile: identity.Profile;
  /** `profile.user`, always present. */
  user: common.UserSummary;
  /** The profile is the viewer's own. */
  isSelf: boolean;
  isFriend: boolean;
}

const contributed: ProfileSection[] = profile.flat();
const SECTIONS = contributed.sort((a, b) => a.order - b.order || a.id.localeCompare(b.id));

/** The sections of one slot, in order. */
export function profileSections(slot: ProfileSlot, sections = SECTIONS): ProfileSection[] {
  return sections.filter((section) => section.slot === slot);
}

/** Every section of a slot, rendered for one profile. */
export function ProfileSlotView({ slot, ...props }: ProfileSectionProps & { slot: ProfileSlot }) {
  return profileSections(slot).map((section) => <section.Component key={section.id} {...props} />);
}
