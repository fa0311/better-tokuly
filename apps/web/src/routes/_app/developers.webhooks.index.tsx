import { createFileRoute } from "@tanstack/react-router";
import { WebhooksScreen } from "#/features/dev/webhooks";

export const Route = createFileRoute("/_app/developers/webhooks/")({
  // `?bot=<id>`: the webhooks of one of the caller's bots instead of their own.
  validateSearch: (search): { bot?: string } => ({
    bot: typeof search.bot === "string" && search.bot ? search.bot : undefined,
  }),
  component: WebhooksPage,
});

function WebhooksPage() {
  const { bot } = Route.useSearch();
  return <WebhooksScreen key={bot ?? ""} botId={bot ?? ""} />;
}
