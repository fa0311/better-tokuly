import { createFileRoute } from "@tanstack/react-router";
import { AdminScreen } from "#/features/emoji/admin/admin-screen";
import { validateAdminSearch } from "#/features/emoji/admin/admin-search";

export const Route = createFileRoute("/_app/admin/emoji")({
  validateSearch: validateAdminSearch,
  component: AdminPage,
});

function AdminPage() {
  return <AdminScreen search={Route.useSearch()} />;
}
