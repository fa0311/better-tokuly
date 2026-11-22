import type { ProfileSection } from "#/seams/profile";
import { ProfilePlans } from "./countdown/profile-cards";

/** ひま right now, pinned countdowns, a couple counter. */
export default [
  {
    id: "plans",
    slot: "card",
    order: 10,
    Component: ({ profile, user, isSelf }) => (
      <ProfilePlans cards={profile.cards} user={user} isSelf={isSelf} className="mt-5" />
    ),
  },
] satisfies ProfileSection[];
