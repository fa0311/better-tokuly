import { createFileRoute } from "@tanstack/react-router";
import { BrowseScreen } from "#/features/emoji/browse-screen";
import { validateBrowseSearch } from "#/features/emoji/browse-search";

export const Route = createFileRoute("/_app/emoji/")({
  // Search, category, order and author live in the address: `/emoji?category=どうぶつ&sort=popular`.
  validateSearch: validateBrowseSearch,
  component: BrowsePage,
});

function BrowsePage() {
  return <BrowseScreen search={Route.useSearch()} />;
}
