import { createFileRoute } from "@tanstack/react-router";
import { RequireAccess } from "#/features/admin/console-layout";
import { CaseScreen } from "#/features/admin/moderation/case-screen";
import { parseQueueSearch } from "#/features/admin/moderation/queue-search";

// The queue's filters ride along in the address so the case keeps its place in the queue.
export const Route = createFileRoute("/_app/admin/moderation/$caseId")({
  validateSearch: parseQueueSearch,
  component: CasePage,
});

function CasePage() {
  const { caseId } = Route.useParams();
  return (
    <RequireAccess level="moderator">
      <CaseScreen caseId={caseId} search={Route.useSearch()} />
    </RequireAccess>
  );
}
