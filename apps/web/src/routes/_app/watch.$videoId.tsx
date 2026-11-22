import { createFileRoute } from "@tanstack/react-router";
import { validateWatchSearch, WatchScreen } from "#/features/video/watch-screen";

/** `/watch/<short id>?t=90&list=<playlist>&comment=<id>`: the address the server uses for videos everywhere. */
export const Route = createFileRoute("/_app/watch/$videoId")({
  validateSearch: validateWatchSearch,
  component: WatchPage,
});

function WatchPage() {
  const { videoId } = Route.useParams();
  return <WatchScreen key={videoId} videoId={videoId} search={Route.useSearch()} />;
}
