import { createFileRoute } from "@tanstack/react-router";
import { SpacesScreen } from "#/features/spaces/spaces-screen";
import { parseSpacesSearch } from "#/features/spaces/spaces-search";

/** Rooms and hangouts. `?tab=hangouts|mine` selects a tab, `?tag=` filters the public rooms. */
export const Route = createFileRoute("/_app/spaces/")({
  validateSearch: parseSpacesSearch,
  component: SpacesPage,
});

function SpacesPage() {
  const { tab, tag } = Route.useSearch();
  return <SpacesScreen tab={tab ?? "discover"} tag={tag ?? ""} />;
}
