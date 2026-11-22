import { createFileRoute } from "@tanstack/react-router";
import { PlanNewScreen } from "#/features/plans/plan/plan-new-screen";

export const Route = createFileRoute("/_app/plans/new")({ component: PlanNewScreen });
