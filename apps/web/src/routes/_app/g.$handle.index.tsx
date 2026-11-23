import { createFileRoute } from "@tanstack/react-router";
import { HomeScreen, parseHomeSearch } from "#/features/community/home-screen";

/** `/g/<handle>`: the community's front page: boards, events, announcements, members, about. */
export const Route = createFileRoute("/_app/g/$handle/")({
  validateSearch: parseHomeSearch,
  component: HomePage,
});

function HomePage() {
  const search = Route.useSearch();
  const navigate = Route.useNavigate();
  return (
    <HomeScreen
      search={search}
      onSearchChange={(next) => void navigate({ search: next, replace: true })}
    />
  );
}
