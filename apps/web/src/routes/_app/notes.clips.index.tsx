import { createFileRoute } from "@tanstack/react-router";
import { ClipsScreen, parseClipsSearch } from "#/features/notes/clips-screen";

/** The caller's clips, or `?user=<handle>`'s public ones. */
export const Route = createFileRoute("/_app/notes/clips/")({
  validateSearch: parseClipsSearch,
  component: ClipsPage,
});

function ClipsPage() {
  const { user } = Route.useSearch();
  return <ClipsScreen key={user ?? ""} handle={user} />;
}
