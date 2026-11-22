import { createFileRoute } from "@tanstack/react-router";
import { DayScreen } from "#/features/history/day-screen";

// `/history/<YYYY-MM-DD>`: one day of the history.
export const Route = createFileRoute("/_app/history/$date")({ component: DayPage });

function DayPage() {
  const { date } = Route.useParams();
  return <DayScreen key={date} date={date} />;
}
