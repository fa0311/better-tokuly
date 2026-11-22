import { createFileRoute } from "@tanstack/react-router";
import { CalendarScreen } from "#/features/plans/calendar/calendar-screen";
import { monthOf, parseYmd, todayYmd } from "#/features/plans/dates";

/** `/plans/calendar?day=YYYY-MM-DD`: the month of that day (today's without it), with the day spelled out. */
export const Route = createFileRoute("/_app/plans/calendar")({
  validateSearch: (search: Record<string, unknown>): { day?: string } =>
    typeof search.day === "string" && parseYmd(search.day) ? { day: search.day } : {},
  component: CalendarPage,
});

function CalendarPage() {
  const search = Route.useSearch();
  const navigate = Route.useNavigate();
  const day = search.day ?? todayYmd();
  return (
    <CalendarScreen
      month={monthOf(day)}
      day={day}
      onChange={(next) =>
        void navigate({ search: next.day === todayYmd() ? {} : { day: next.day }, replace: true })
      }
    />
  );
}
