import { createFileRoute } from "@tanstack/react-router";
import { MapScreen } from "#/features/map/map-screen";

/**
 * `/map`: the live friend map. `?friend=<user id>` opens that friend's sheet
 * and flies to their pin (links from notifications and profiles).
 */
export const Route = createFileRoute("/_app/map")({
  validateSearch: (search: Record<string, unknown>): { friend?: string } =>
    typeof search.friend === "string" && search.friend ? { friend: search.friend } : {},
  component: MapPage,
});

function MapPage() {
  const { friend } = Route.useSearch();
  return <MapScreen focusUserId={friend} />;
}
