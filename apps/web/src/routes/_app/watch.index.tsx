import { createFileRoute } from "@tanstack/react-router";
import { validateFeedSearch } from "@tokuly/core/video/search";
import { FeedsScreen } from "#/features/video/feeds-screen";

/** The video feeds. The open feed and genre live in the address: `/watch?feed=trending&category=1`. */
export const Route = createFileRoute("/_app/watch/")({
  validateSearch: validateFeedSearch,
  component: FeedsPage,
});

function FeedsPage() {
  const search = Route.useSearch();
  const navigate = Route.useNavigate();
  return (
    <FeedsScreen
      search={search}
      onSearchChange={(next) => navigate({ search: next, replace: true })}
    />
  );
}
