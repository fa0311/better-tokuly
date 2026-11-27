import { createFileRoute } from "@tanstack/react-router";
import { ChannelsScreen } from "#/features/notes/channels-screen";
import { parseChannelsSearch } from "#/features/notes/channels-search";

/** The channel directory. `?tab=followed|managed` and `?q=` keep the list on going back. */
export const Route = createFileRoute("/_app/notes/channels/")({
  validateSearch: parseChannelsSearch,
  component: ChannelsPage,
});

function ChannelsPage() {
  const { tab, q } = Route.useSearch();
  const navigate = Route.useNavigate();
  return (
    <ChannelsScreen
      tab={tab}
      query={q}
      onChange={(search) => void navigate({ search, replace: true })}
    />
  );
}
