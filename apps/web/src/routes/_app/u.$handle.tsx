import { createFileRoute } from "@tanstack/react-router";
import { ProfileScreen } from "#/features/profile/profile-screen";

export const Route = createFileRoute("/_app/u/$handle")({ component: ProfilePage });

function ProfilePage() {
  const { handle } = Route.useParams();
  return <ProfileScreen key={handle} handle={handle} />;
}
