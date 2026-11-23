import { createFileRoute } from "@tanstack/react-router";
import { HangoutScreen } from "#/features/spaces/hangout/hangout-screen";

export const Route = createFileRoute("/_app/spaces/hangouts/$hangoutId")({
  component: HangoutPage,
});

function HangoutPage() {
  const { hangoutId } = Route.useParams();
  return <HangoutScreen key={hangoutId} hangoutId={hangoutId} />;
}
