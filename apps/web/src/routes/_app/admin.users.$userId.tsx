import { createFileRoute } from "@tanstack/react-router";
import { RequireAccess } from "#/features/admin/console-layout";
import { UserScreen } from "#/features/admin/users/user-screen";

export const Route = createFileRoute("/_app/admin/users/$userId")({ component: UserPage });

function UserPage() {
  const { userId } = Route.useParams();
  return (
    <RequireAccess level="moderator">
      <UserScreen key={userId} userId={userId} />
    </RequireAccess>
  );
}
