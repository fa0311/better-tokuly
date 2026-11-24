import type { ProfileSection } from "#/seams/profile";
import { ProfileClipsLink } from "./clips-screen";
import { ProfileNotes } from "./profile-notes";

/** The person's notes, with their tabs. */
export default [
  {
    id: "notes",
    slot: "content",
    order: 20,
    Component: ({ user, isSelf }) => <ProfileNotes userId={user.id} isSelf={isSelf} />,
  },
  { id: "notes-clips", slot: "action", order: 60, Component: ProfileClipsLink },
] satisfies ProfileSection[];
