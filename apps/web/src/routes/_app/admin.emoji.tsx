import { createFileRoute } from "@tanstack/react-router";
import { AdminScreen, validateAdminSearch } from "#/features/emoji/admin/admin-screen";

export const Route = createFileRoute("/_app/admin/emoji")({ validateSearch: validateAdminSearch, component: AdminPage });

function AdminPage() {
  return <AdminScreen search={Route.useSearch()} />;
}
