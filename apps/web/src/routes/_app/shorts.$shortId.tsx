import { createFileRoute } from "@tanstack/react-router";
import { contextOfSearch, validateShortSearch } from "@tokuly/core/shorts/search";
import { ShortScreen } from "#/features/shorts/feed/feed-screen";

/**
 * One short, by its link: the swipe feed opened on it. `?list=tag&key=ねこ`
 * says which grid it was tapped in, and swiping goes on through that list.
 */
export const Route = createFileRoute("/_app/shorts/$shortId")({
  validateSearch: validateShortSearch,
  component: ShortPage,
});

function ShortPage() {
  const { shortId } = Route.useParams();
  const search = Route.useSearch();
  return <ShortScreen shortId={shortId} context={contextOfSearch(search)} />;
}
