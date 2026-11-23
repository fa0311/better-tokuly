import { createFileRoute } from "@tanstack/react-router";
import { StreamAnalyticsScreen } from "#/features/live/studio/analytics";

export const Route = createFileRoute("/_app/studio/live/$streamId/analytics")({
  component: AnalyticsPage,
});

function AnalyticsPage() {
  const { streamId } = Route.useParams();
  return <StreamAnalyticsScreen key={streamId} streamId={streamId} />;
}
