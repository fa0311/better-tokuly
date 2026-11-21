import { createFileRoute } from "@tanstack/react-router";
import { AnnouncementScreen } from "#/features/announcements/announcement-screen";

export const Route = createFileRoute("/_app/announcements/$announcementId")({ component: AnnouncementPage });

function AnnouncementPage() {
  const { announcementId } = Route.useParams();
  return <AnnouncementScreen key={announcementId} announcementId={announcementId} />;
}
