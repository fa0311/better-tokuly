import { createFileRoute } from "@tanstack/react-router";
import { CommunityLayout } from "#/features/community/community-layout";

/**
 * Frame of a community's pages (`/g/<handle>/…`): loads the community,
 * follows its realtime topic and shows what a visitor who may not read it
 * gets instead. (`/c/<handle>` is a video channel in this app.)
 */
export const Route = createFileRoute("/_app/g/$handle")({ component: CommunityPage });

function CommunityPage() {
  const { handle } = Route.useParams();
  return <CommunityLayout key={handle.toLowerCase()} handle={handle} />;
}
