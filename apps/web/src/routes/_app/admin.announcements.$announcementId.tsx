import { createFileRoute } from "@tanstack/react-router";
import { AnnouncementEditorScreen } from "#/features/admin/announcements/editor-screen";
import { RequireAccess } from "#/features/admin/console-layout";

export const Route = createFileRoute("/_app/admin/announcements/$announcementId")({ component: EditAnnouncementPage });

function EditAnnouncementPage() {
  const { announcementId } = Route.useParams();
  return (
    <RequireAccess level="admin">
      <AnnouncementEditorScreen key={announcementId} announcementId={announcementId} />
    </RequireAccess>
  );
}
