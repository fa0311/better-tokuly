import { createFileRoute } from "@tanstack/react-router";
import { EditChannelScreen } from "#/features/video/studio/channel-form";

export const Route = createFileRoute("/_app/studio/channels/$channelId")({
  component: EditChannelPage,
});

function EditChannelPage() {
  const { channelId } = Route.useParams();
  return <EditChannelScreen key={channelId} channelId={channelId} />;
}
