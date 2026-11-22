import { createFileRoute } from "@tanstack/react-router";
import { BookmarksScreen, parseBookmarksSearch } from "#/features/engagement/bookmarks-screen";

/** Saved subjects and collections. `?collection=<id>` opens one collection, `?tab=reactions` what the viewer reacted to. */
export const Route = createFileRoute("/_app/bookmarks/")({
  validateSearch: parseBookmarksSearch,
  component: BookmarksPage,
});

function BookmarksPage() {
  const search = Route.useSearch();
  const navigate = Route.useNavigate();
  return (
    <BookmarksScreen
      search={search}
      onSearchChange={(next) => void navigate({ search: next, replace: true })}
    />
  );
}
