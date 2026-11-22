import { createFileRoute } from "@tanstack/react-router";
import { parseSearchParams } from "#/features/search/params";
import { SearchScreen } from "#/features/search/search-screen";

// `/search?q=…&kind=…&sort=…&owner=…`: the address says everything the page shows.
export const Route = createFileRoute("/_app/search")({
  validateSearch: (search: Record<string, unknown>) => parseSearchParams(search),
  component: SearchPage,
});

function SearchPage() {
  return <SearchScreen params={Route.useSearch()} />;
}
