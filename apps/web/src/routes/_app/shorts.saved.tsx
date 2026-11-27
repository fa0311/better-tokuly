import { createFileRoute } from "@tanstack/react-router";
import { SavedScreen } from "#/features/shorts/saved-screen";
import { validateSavedSearch } from "#/features/shorts/saved-search";

export const Route = createFileRoute("/_app/shorts/saved")({
  validateSearch: validateSavedSearch,
  component: SavedPage,
});

function SavedPage() {
  const { tab } = Route.useSearch();
  const navigate = Route.useNavigate();
  return (
    <SavedScreen
      tab={tab ?? "saved"}
      onTabChange={(next) =>
        navigate({ search: next === "liked" ? { tab: "liked" } : {}, replace: true })
      }
    />
  );
}
