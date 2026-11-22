import { createFileRoute } from "@tanstack/react-router";
import { EditPlaceScreen } from "#/features/places/place-editor-screen";

export const Route = createFileRoute("/_app/places/$placeId/edit")({ component: EditPlacePage });

function EditPlacePage() {
  const { placeId } = Route.useParams();
  return <EditPlaceScreen key={placeId} placeId={placeId} />;
}
