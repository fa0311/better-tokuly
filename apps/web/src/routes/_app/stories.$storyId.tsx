import { createFileRoute } from "@tanstack/react-router";
import { StoryScreen } from "#/features/stories/story-screen";

// `/stories/<id>`: the address the server uses for a story in notifications, the feed and reports.
export const Route = createFileRoute("/_app/stories/$storyId")({ component: StoryPage });

function StoryPage() {
  const { storyId } = Route.useParams();
  return <StoryScreen key={storyId} storyId={storyId} />;
}
