import { createFileRoute } from "@tanstack/react-router";
import { AnnouncementEditorScreen } from "#/features/admin/announcements/editor-screen";
import { RequireAccess } from "#/features/admin/console-layout";

export const Route = createFileRoute("/_app/admin/announcements/new")({
  component: NewAnnouncementPage,
});

function NewAnnouncementPage() {
  return (
    <RequireAccess level="admin">
      <AnnouncementEditorScreen />
    </RequireAccess>
  );
}
