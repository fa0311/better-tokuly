import { createFileRoute } from "@tanstack/react-router";
import { AuditScreen, parseAuditSearch } from "#/features/admin/audit/audit-screen";
import { RequireAccess } from "#/features/admin/console-layout";

export const Route = createFileRoute("/_app/admin/audit")({ validateSearch: parseAuditSearch, component: AuditPage });

function AuditPage() {
  return (
    <RequireAccess level="admin">
      <AuditScreen search={Route.useSearch()} />
    </RequireAccess>
  );
}
