import { createFileRoute } from "@tanstack/react-router";
import { ChannelTabPage } from "#/features/video/channel/channel-layout";

// `/c/<handle>/<tab>`: videos, playlists, about — and the tabs other features register (see `channel/tabs.tsx`).
export const Route = createFileRoute("/_app/c/$handle/$tab")({ component: TabPage });

function TabPage() {
  const { handle, tab } = Route.useParams();
  return <ChannelTabPage handle={handle} tab={tab} />;
}
