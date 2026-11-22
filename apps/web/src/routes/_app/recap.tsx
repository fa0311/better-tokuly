import { createFileRoute } from "@tanstack/react-router";
import { parseRecapSearch, RecapScreen } from "#/features/notes/feed/recap-screen";

/** The recap of a month or a year: `?p=2026-10`, `?p=2026`; without it, this month so far. */
export const Route = createFileRoute("/_app/recap")({
  validateSearch: parseRecapSearch,
  component: RecapPage,
});

function RecapPage() {
  const search = Route.useSearch();
  const navigate = Route.useNavigate();
  return (
    <RecapScreen
      search={search}
      onSearchChange={(next) => void navigate({ search: next, replace: true })}
    />
  );
}
