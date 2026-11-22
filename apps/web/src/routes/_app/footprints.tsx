import { createFileRoute } from "@tanstack/react-router";
import { FootprintsScreen } from "#/features/footprints/footprints-screen";

// `/footprints`: my scratch map; `/footprints?user=<id>`: a friend's.
export const Route = createFileRoute("/_app/footprints")({
  validateSearch: (search: Record<string, unknown>): { user?: string } => (typeof search.user === "string" && search.user ? { user: search.user } : {}),
  component: FootprintsPage,
});

function FootprintsPage() {
  const { user } = Route.useSearch();
  return <FootprintsScreen userId={user} />;
}
