import { createFileRoute } from "@tanstack/react-router";
import { BadgeScreen } from "#/features/quest/badge-screen";

// `/quest/badges/<key>`: the address the server uses for a badge in notifications.
export const Route = createFileRoute("/_app/quest/badges/$key")({ component: BadgePage });

function BadgePage() {
  const { key } = Route.useParams();
  return <BadgeScreen key={key} badgeKey={key} />;
}
