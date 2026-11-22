import { createFileRoute } from "@tanstack/react-router";
import { BotScreen } from "#/features/dev/bots";

export const Route = createFileRoute("/_app/developers/bots/$botId")({ component: BotPage });

function BotPage() {
  const { botId } = Route.useParams();
  return <BotScreen key={botId} botId={botId} />;
}
