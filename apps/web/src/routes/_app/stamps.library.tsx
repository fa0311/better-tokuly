import { createFileRoute } from "@tanstack/react-router";
import { LibraryScreen, validateLibrarySearch } from "#/features/stamps/library-screen";

export const Route = createFileRoute("/_app/stamps/library")({
  // `/stamps/library?tab=log` opens a tab directly.
  validateSearch: validateLibrarySearch,
  component: LibraryPage,
});

function LibraryPage() {
  const { tab } = Route.useSearch();
  return <LibraryScreen tab={tab ?? "packs"} />;
}
