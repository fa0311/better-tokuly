import { createFileRoute } from "@tanstack/react-router";
import { DiscoverScreen, parseDiscoverSearch } from "#/features/community/discover-screen";

/**
 * `/communities`: my communities and the directory. The tab, the search
 * text and the filters live in the address, so going back returns to them.
 */
export const Route = createFileRoute("/_app/communities/")({
  validateSearch: parseDiscoverSearch,
  component: CommunitiesPage,
});

function CommunitiesPage() {
  const search = Route.useSearch();
  const navigate = Route.useNavigate();
  return (
    <DiscoverScreen
      search={search}
      onSearchChange={(next) => void navigate({ search: next, replace: true })}
    />
  );
}
