import { createFileRoute } from "@tanstack/react-router";
import { RequireAccess } from "#/features/admin/console-layout";
import { ModerationQueueScreen } from "#/features/admin/moderation/queue-screen";
import { parseQueueSearch } from "#/features/admin/moderation/queue-search";

export const Route = createFileRoute("/_app/admin/moderation/")({
  validateSearch: parseQueueSearch,
  component: QueuePage,
});

function QueuePage() {
  return (
    <RequireAccess level="moderator">
      <ModerationQueueScreen search={Route.useSearch()} />
    </RequireAccess>
  );
}
