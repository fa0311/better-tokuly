import { createFileRoute } from "@tanstack/react-router";
import { CountdownScreen } from "#/features/plans/countdown/countdown-screen";

export const Route = createFileRoute("/_app/plans/countdowns/$countdownId")({
  component: CountdownPage,
});

function CountdownPage() {
  const { countdownId } = Route.useParams();
  return <CountdownScreen key={countdownId} countdownId={countdownId} />;
}
