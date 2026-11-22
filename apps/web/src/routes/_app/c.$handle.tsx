import { createFileRoute } from "@tanstack/react-router";
import { ChannelLayout } from "#/features/video/channel/channel-layout";

// Frame of a channel page: the header, and the tabs as the routes below it.
export const Route = createFileRoute("/_app/c/$handle")({ component: ChannelPage });

function ChannelPage() {
  const { handle } = Route.useParams();
  return <ChannelLayout key={handle} handle={handle} />;
}
