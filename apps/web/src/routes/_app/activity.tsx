import { createFileRoute } from "@tanstack/react-router";
import { ActivityScreen } from "#/features/notes/feed/activity-screen";
import { parseActivitySearch } from "#/features/notes/feed/activity-search";

/** The log. `?tab=friends` is what friends have been doing, `?tab=reports` the viewer's reports. */
export const Route = createFileRoute("/_app/activity")({
  validateSearch: parseActivitySearch,
  component: ActivityPage,
});

function ActivityPage() {
  const { tab = "mine" } = Route.useSearch();
  const navigate = Route.useNavigate();
  return (
    <ActivityScreen
      tab={tab}
      onTabChange={(next) =>
        void navigate({ search: next === "mine" ? {} : { tab: next }, replace: true })
      }
    />
  );
}
