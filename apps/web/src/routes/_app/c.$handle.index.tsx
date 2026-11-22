import { createFileRoute } from "@tanstack/react-router";
import { ChannelHome } from "#/features/video/channel/channel-home";

export const Route = createFileRoute("/_app/c/$handle/")({ component: HomeTab });

function HomeTab() {
  const { handle } = Route.useParams();
  return <ChannelHome handle={handle} />;
}
