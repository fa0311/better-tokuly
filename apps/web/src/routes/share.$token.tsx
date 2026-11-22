import { createFileRoute } from "@tanstack/react-router";
import { ShareScreen } from "#/features/map/share-screen";

/**
 * `/share/<token>`: the public page of a live share. Whoever has the link
 * sees the sharer's moving pin without an account, so it lives outside the
 * signed-in layout and renders in the browser only (the map needs WebGL).
 */
export const Route = createFileRoute("/share/$token")({ ssr: false, component: SharePage });

function SharePage() {
  const { token } = Route.useParams();
  return <ShareScreen key={token} token={token} />;
}
