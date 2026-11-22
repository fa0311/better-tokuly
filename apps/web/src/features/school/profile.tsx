import type { ProfileSection } from "#/seams/profile";
import { SchoolProfileCards } from "./profile-cards";

/** 勉強中, the class that is on, テスト期間: what school says about the person right now. */
export default [
  {
    id: "school",
    slot: "status",
    order: 10,
    Component: ({ profile, user, isSelf }) => (
      <SchoolProfileCards cards={profile.cards} user={user} isSelf={isSelf} />
    ),
  },
] satisfies ProfileSection[];
