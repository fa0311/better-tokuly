import type { ProfileSection } from "#/seams/profile";
import { ProfileAnswers, ProfileAskButton } from "./profile-ask";

/** The question box on a profile: the way in among the actions, the newest answers further down. */
export default [
  {
    id: "ask-button",
    slot: "action",
    order: 10,
    Component: ({ user, isSelf }) => <ProfileAskButton user={user} isSelf={isSelf} />,
  },
  {
    id: "ask-answers",
    slot: "content",
    order: 10,
    Component: ({ user }) => <ProfileAnswers user={user} />,
  },
] satisfies ProfileSection[];
