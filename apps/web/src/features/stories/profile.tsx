import type { ProfileSection } from "#/seams/profile";
import { ProfileHighlights } from "./highlights";

/** The person's highlights (stories they kept). */
export default [
  {
    id: "stories-highlights",
    slot: "card",
    order: 20,
    Component: ({ user, isSelf }) => <ProfileHighlights user={user} isSelf={isSelf} />,
  },
] satisfies ProfileSection[];
