import { createFileRoute } from "@tanstack/react-router";
import { PackScreen } from "#/features/stamps/pack-screen";

// `/stamps/<short id>`: the address the server uses for packs in notifications, the feed and search.
export const Route = createFileRoute("/_app/stamps/$packId")({ component: PackPage });

function PackPage() {
  const { packId } = Route.useParams();
  return <PackScreen key={packId} packId={packId} />;
}
