import { createFileRoute } from "@tanstack/react-router";
import { FriendTimetableScreen } from "#/features/school/friends-screen";

// `/school/timetable/<user id>`: a friend's timetable as they share it (the address their profile card uses).
export const Route = createFileRoute("/_app/school/timetable/$userId")({ component: FriendTimetablePage });

function FriendTimetablePage() {
  const { userId } = Route.useParams();
  return <FriendTimetableScreen key={userId} userId={userId} />;
}
