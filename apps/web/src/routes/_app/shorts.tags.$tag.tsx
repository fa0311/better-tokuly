import { createFileRoute } from "@tanstack/react-router";
import { HashtagScreen } from "#/features/shorts/hashtag-screen";

// `/shorts/tags/<tag>`: where a hashtag in a caption and a hashtag search result lead.
export const Route = createFileRoute("/_app/shorts/tags/$tag")({ component: HashtagPage });

function HashtagPage() {
  const { tag } = Route.useParams();
  return <HashtagScreen key={tag} tag={tag} />;
}
