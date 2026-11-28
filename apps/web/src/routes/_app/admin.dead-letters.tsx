import { createFileRoute } from "@tanstack/react-router";
import { RequireAccess } from "#/features/admin/console-layout";
import { DeadLettersScreen } from "#/features/admin/dead-letters/dead-letters-screen";
import { parseDeadLettersSearch } from "#/features/admin/dead-letters/dead-letters-search";

export const Route = createFileRoute("/_app/admin/dead-letters")({
  validateSearch: parseDeadLettersSearch,
  component: DeadLettersPage,
});

function DeadLettersPage() {
  return (
    <RequireAccess level="admin">
      <DeadLettersScreen search={Route.useSearch()} />
    </RequireAccess>
  );
}
