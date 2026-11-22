import { createFileRoute } from "@tanstack/react-router";
import {
  AdminSearchScreen,
  STAFF_TABS,
  type StaffTab,
} from "#/features/search/admin/admin-search-screen";

export const Route = createFileRoute("/_app/admin/search")({
  // The open tab lives in the address: `/admin/search?tab=blocked`.
  validateSearch: (search: Record<string, unknown>): { tab?: StaffTab } => ({
    tab: STAFF_TABS.find((tab) => tab === search.tab),
  }),
  component: AdminSearchPage,
});

function AdminSearchPage() {
  const { tab } = Route.useSearch();
  const navigate = Route.useNavigate();
  return (
    <AdminSearchScreen
      tab={tab ?? "indexes"}
      onTab={(next) => void navigate({ search: { tab: next === "indexes" ? undefined : next } })}
    />
  );
}
