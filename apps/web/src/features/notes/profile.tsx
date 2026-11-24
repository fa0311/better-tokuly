import type { ProfileSection } from "#/seams/profile";
import { AddToListButton, ProfileLists } from "./lists-profile";
import { ProfileNotes } from "./profile-notes";

/** The person's notes with their tabs, their public lists, and "add to list". */
export default [
  {
    id: "notes",
    slot: "content",
    order: 20,
    Component: ({ user, isSelf }) => <ProfileNotes userId={user.id} isSelf={isSelf} />,
  },
  {
    id: "notes-add-to-list",
    slot: "action",
    order: 60,
    Component: ({ user, isSelf }) => <AddToListButton user={user} isSelf={isSelf} />,
  },
  {
    id: "notes-lists",
    slot: "content",
    order: 30,
    Component: ({ user }) => <ProfileLists user={user} />,
  },
] satisfies ProfileSection[];
