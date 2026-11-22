import { createFileRoute } from "@tanstack/react-router";
import { PlacesScreen, type PlacesTab } from "#/features/places/places-screen";

// `/places?tab=saved|search&q=…`: my places, saved places, the place search.
export const Route = createFileRoute("/_app/places/")({
  validateSearch: (search: Record<string, unknown>): { tab?: PlacesTab; q?: string } => ({
    ...(search.tab === "saved" || search.tab === "search" ? { tab: search.tab } : {}),
    ...(typeof search.q === "string" && search.q ? { q: search.q.slice(0, 100) } : {}),
  }),
  component: PlacesPage,
});

function PlacesPage() {
  const { tab, q } = Route.useSearch();
  return <PlacesScreen tab={tab ?? "mine"} query={q ?? ""} />;
}
