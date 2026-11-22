import { createFileRoute } from "@tanstack/react-router";
import { PlanScreen } from "#/features/plans/plan/plan-screen";

// `/plans/<id>`: the address the server uses for a plan in notifications, the calendar and reports.
export const Route = createFileRoute("/_app/plans/$planId")({ component: PlanPage });

function PlanPage() {
  const { planId } = Route.useParams();
  return <PlanScreen key={planId} planId={planId} />;
}
