import type { ProfileSection, ProfileSectionProps } from "#/seams/profile";
import { HangoutPresence } from "./presence";

/** Under a friend's bio: the room or hangout they are in right now, as a way to drop in. */
function InRoomNow({ user, isSelf, isFriend }: ProfileSectionProps) {
  // Only friends can be shown in a room; nobody else is asked about.
  if (isSelf || !isFriend) return null;
  return (
    <div className="mt-3 flex justify-center">
      <HangoutPresence userId={user.id} className="h-8 px-3.5 text-sm" />
    </div>
  );
}

export default [
  { id: "spaces", slot: "status", order: 40, Component: InRoomNow },
] satisfies ProfileSection[];
