import { createFileRoute } from "@tanstack/react-router";
import { LibraryScreen } from "#/features/stamps/library-screen";
import { validateLibrarySearch } from "#/features/stamps/library-search";

export const Route = createFileRoute("/_app/stamps/library")({
  // `/stamps/library?tab=log` opens a tab directly.
  validateSearch: validateLibrarySearch,
  component: LibraryPage,
});

function LibraryPage() {
  const { tab } = Route.useSearch();
  return <LibraryScreen tab={tab ?? "packs"} />;
}
