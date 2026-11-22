import { createFileRoute } from "@tanstack/react-router";
import { TimetableScreen } from "#/features/school/timetable-screen";

/** The caller's weekly timetable. `?term=<id>` shows another term than the current one. */
export const Route = createFileRoute("/_app/school/timetable/")({
  validateSearch: (search: Record<string, unknown>): { term?: string } => (typeof search.term === "string" && search.term ? { term: search.term } : {}),
  component: TimetablePage,
});

function TimetablePage() {
  const { term } = Route.useSearch();
  const navigate = Route.useNavigate();
  return <TimetableScreen termId={term} onTermChange={(next) => void navigate({ search: { term: next }, replace: true })} />;
}
