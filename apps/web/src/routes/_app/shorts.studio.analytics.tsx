import { createFileRoute } from "@tanstack/react-router";
import { AnalyticsScreen } from "#/features/shorts/studio/analytics-screen";
import { validateAnalyticsSearch } from "#/features/shorts/studio/analytics-search";

/** Creator numbers: the channel's shorts, or one short with `?short=<id>`; `?days=7|90` for the range. */
export const Route = createFileRoute("/_app/shorts/studio/analytics")({
  validateSearch: validateAnalyticsSearch,
  component: AnalyticsPage,
});

function AnalyticsPage() {
  const search = Route.useSearch();
  const navigate = Route.useNavigate();
  return (
    <AnalyticsScreen
      search={search}
      onSearchChange={(next) => navigate({ search: next, replace: true })}
    />
  );
}
