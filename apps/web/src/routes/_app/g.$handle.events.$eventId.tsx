import { createFileRoute } from "@tanstack/react-router";
import { EventScreen } from "#/features/community/event-screen";

/** `/g/<handle>/events/<event>`: an event: answer, who is coming, check-in. */
export const Route = createFileRoute("/_app/g/$handle/events/$eventId")({ component: EventPage });

function EventPage() {
  const { eventId } = Route.useParams();
  return <EventScreen key={eventId} eventId={eventId} />;
}
