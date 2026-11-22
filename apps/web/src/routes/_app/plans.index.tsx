import { createFileRoute } from "@tanstack/react-router";
import { PlansScreen, parsePlansSearch } from "#/features/plans/plans-screen";

/** The plans hub. `?tab=polls|moments` selects a tab, so going back returns to it. */
export const Route = createFileRoute("/_app/plans/")({ validateSearch: parsePlansSearch, component: PlansPage });

function PlansPage() {
  const { tab } = Route.useSearch();
  const navigate = Route.useNavigate();
  return <PlansScreen tab={tab ?? "plans"} onTabChange={(next) => void navigate({ search: next === "plans" ? {} : { tab: next }, replace: true })} />;
}
