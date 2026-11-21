import { createFileRoute } from "@tanstack/react-router";
import { RequireAccess } from "#/features/admin/console-layout";
import { FlagsScreen, parseFlagsSearch } from "#/features/admin/flags/flags-screen";

export const Route = createFileRoute("/_app/admin/flags")({ validateSearch: parseFlagsSearch, component: FlagsPage });

function FlagsPage() {
  return (
    <RequireAccess level="admin">
      <FlagsScreen search={Route.useSearch()} />
    </RequireAccess>
  );
}
