import { createFileRoute } from "@tanstack/react-router";
import { ChannelPage } from "#/features/notes/channels-page";

// `/notes/channels/<id>`: the address the server uses for a channel (search results, notifications).
export const Route = createFileRoute("/_app/notes/channels/$channelId")({ component: Channel });

function Channel() {
  const { channelId } = Route.useParams();
  return <ChannelPage key={channelId} channelId={channelId} />;
}
