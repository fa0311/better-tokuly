import { createFileRoute } from "@tanstack/react-router";
import { CreatorScreen } from "#/features/stamps/creator-screen";

export const Route = createFileRoute("/_app/stamps/creator/$userId")({ component: CreatorPage });

function CreatorPage() {
  const { userId } = Route.useParams();
  return <CreatorScreen key={userId} userId={userId} />;
}
