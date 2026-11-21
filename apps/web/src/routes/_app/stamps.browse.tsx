import { createFileRoute } from "@tanstack/react-router";
import { BrowseScreen, validateBrowseSearch } from "#/features/stamps/browse-screen";

export const Route = createFileRoute("/_app/stamps/browse")({
  // The search, shelf, tag and price filter live in the address:
  // `/stamps/browse?tag=ねこ&price=free`.
  validateSearch: validateBrowseSearch,
  component: BrowsePage,
});

function BrowsePage() {
  return <BrowseScreen search={Route.useSearch()} />;
}
