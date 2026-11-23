import { createFileRoute } from "@tanstack/react-router";
import { AppScreen } from "#/features/dev/apps";

export const Route = createFileRoute("/_app/developers/apps/$appId")({ component: AppPage });

function AppPage() {
  const { appId } = Route.useParams();
  return <AppScreen key={appId} appId={appId} />;
}
