import { createFileRoute } from "@tanstack/react-router";
import { ManagedAnnouncementsScreen, parseManagedAnnouncementsSearch } from "#/features/admin/announcements/list-screen";
import { RequireAccess } from "#/features/admin/console-layout";

export const Route = createFileRoute("/_app/admin/announcements/")({ validateSearch: parseManagedAnnouncementsSearch, component: AnnouncementsPage });

function AnnouncementsPage() {
  return (
    <RequireAccess level="admin">
      <ManagedAnnouncementsScreen search={Route.useSearch()} />
    </RequireAccess>
  );
}
