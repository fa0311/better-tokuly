import { createFileRoute } from "@tanstack/react-router";
import { XpScreen, type XpTab } from "#/features/quest/xp-screen";

export const Route = createFileRoute("/_app/quest/xp")({
  validateSearch: (search: Record<string, unknown>): { tab?: XpTab } =>
    search.tab === "history" ? { tab: "history" } : {},
  component: XpPage,
});

function XpPage() {
  const { tab } = Route.useSearch();
  const navigate = Route.useNavigate();
  return (
    <XpScreen
      tab={tab ?? "rules"}
      onTab={(next) =>
        void navigate({ search: next === "history" ? { tab: next } : {}, replace: true })
      }
    />
  );
}
