import { createFileRoute } from "@tanstack/react-router";
import { PlaceScreen } from "#/features/places/place-screen";

// `/places/<id>`: the address the server uses for a place in search, notifications and share links.
export const Route = createFileRoute("/_app/places/$placeId/")({ component: PlacePage });

function PlacePage() {
  const { placeId } = Route.useParams();
  return <PlaceScreen key={placeId} placeId={placeId} />;
}
