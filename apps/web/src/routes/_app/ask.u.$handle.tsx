import { createFileRoute } from "@tanstack/react-router";
import { BoxScreen } from "#/features/ask/box-screen";

/** `/ask/u/<handle>`: a person's question box inside the app. */
export const Route = createFileRoute("/_app/ask/u/$handle")({ component: BoxPage });

function BoxPage() {
  const { handle } = Route.useParams();
  return <BoxScreen key={handle} handle={handle} />;
}
