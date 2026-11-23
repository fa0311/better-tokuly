import { createFileRoute } from "@tanstack/react-router";
import { WebhookScreen } from "#/features/dev/webhooks";

export const Route = createFileRoute("/_app/developers/webhooks/$webhookId")({
  component: WebhookPage,
});

function WebhookPage() {
  const { webhookId } = Route.useParams();
  return <WebhookScreen key={webhookId} webhookId={webhookId} />;
}
