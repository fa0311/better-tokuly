import { createFileRoute } from "@tanstack/react-router";
import { BrowseLiveScreen, type BrowseTab } from "#/features/live/browse-screen";

// `/live`: what is on air and what is scheduled (`?tab=upcoming`).
export const Route = createFileRoute("/_app/live/")({
  validateSearch: (search: Record<string, unknown>): { tab?: BrowseTab } =>
    search.tab === "upcoming" ? { tab: "upcoming" } : {},
  component: BrowsePage,
});

function BrowsePage() {
  const { tab } = Route.useSearch();
  const navigate = Route.useNavigate();
  return (
    <BrowseLiveScreen
      tab={tab ?? "now"}
      onTabChange={(next) =>
        void navigate({ search: next === "upcoming" ? { tab: next } : {}, replace: true })
      }
    />
  );
}
