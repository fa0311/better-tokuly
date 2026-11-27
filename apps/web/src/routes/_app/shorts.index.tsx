import { createFileRoute } from "@tanstack/react-router";
import { FeedScreen } from "#/features/shorts/feed/feed-screen";
import { validateFeedSearch } from "#/features/shorts/feed/feed-search";

/** The swipe feed. The open feed lives in the address: `/shorts?tab=following`. */
export const Route = createFileRoute("/_app/shorts/")({
  validateSearch: validateFeedSearch,
  component: FeedPage,
});

function FeedPage() {
  const search = Route.useSearch();
  const navigate = Route.useNavigate();
  return (
    <FeedScreen
      search={search}
      onSearchChange={(next) => navigate({ search: next, replace: true })}
    />
  );
}
