import { createFileRoute } from "@tanstack/react-router";
import { RequireAccess } from "#/features/admin/console-layout";
import { UsersScreen } from "#/features/admin/users/users-screen";
import { parseUsersSearch } from "#/features/admin/users/users-search";

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
