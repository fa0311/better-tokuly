import { createFileRoute } from "@tanstack/react-router";
import { RequireAccess } from "#/features/admin/console-layout";
import { ModerationLogScreen, parseLogSearch } from "#/features/admin/moderation/log-screen";

export const Route = createFileRoute("/_app/admin/moderation/log")({ validateSearch: parseLogSearch, component: LogPage });

function LogPage() {
  return (
    <RequireAccess level="moderator">
      <ModerationLogScreen search={Route.useSearch()} />
    </RequireAccess>
  );
}
