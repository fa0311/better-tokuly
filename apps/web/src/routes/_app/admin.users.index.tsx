import { createFileRoute } from "@tanstack/react-router";
import { RequireAccess } from "#/features/admin/console-layout";
import { parseUsersSearch, UsersScreen } from "#/features/admin/users/users-screen";

export const Route = createFileRoute("/_app/admin/users/")({
  validateSearch: parseUsersSearch,
  component: UsersPage,
});

function UsersPage() {
  return (
    <RequireAccess level="moderator">
      <UsersScreen search={Route.useSearch()} />
    </RequireAccess>
  );
}
