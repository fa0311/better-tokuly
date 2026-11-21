import { createFileRoute } from "@tanstack/react-router";
import { StudioPackScreen } from "#/features/stamps/studio-pack-screen";

// Where the creator's notifications about a pack lead (rejected, taken down, sold).
export const Route = createFileRoute("/_app/stamps/studio/$packId")({ component: StudioPackPage });

function StudioPackPage() {
  const { packId } = Route.useParams();
  return <StudioPackScreen key={packId} packId={packId} />;
}
