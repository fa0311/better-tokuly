import { createFileRoute } from "@tanstack/react-router";
import { MomentScreen } from "#/features/moments/moment-screen";

export const Route = createFileRoute("/_app/moments/$momentId")({ component: MomentRoute });

function MomentRoute() {
  const { momentId } = Route.useParams();
  return <MomentScreen key={momentId} momentId={momentId} />;
}
