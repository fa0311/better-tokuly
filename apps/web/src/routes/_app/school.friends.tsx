import { createFileRoute } from "@tanstack/react-router";
import { type FriendsTab, SchoolFriendsScreen } from "#/features/school/friends-screen";

/** Friends' timetables and shared free periods. `?tab=free` opens on 空きコマ. */
export const Route = createFileRoute("/_app/school/friends")({
  validateSearch: (search: Record<string, unknown>): { tab?: FriendsTab } => (search.tab === "free" ? { tab: "free" } : {}),
  component: FriendsPage,
});

function FriendsPage() {
  const { tab } = Route.useSearch();
  const navigate = Route.useNavigate();
  return <SchoolFriendsScreen tab={tab ?? "timetables"} onTabChange={(next) => void navigate({ search: next === "free" ? { tab: "free" } : {}, replace: true })} />;
}
