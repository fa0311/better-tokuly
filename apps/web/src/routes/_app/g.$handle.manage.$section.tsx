import { createFileRoute } from "@tanstack/react-router";
import { ManageSectionScreen } from "#/features/community/manage/manage-screen";

/** `/g/<handle>/manage/<section>`: one of the staff pages (settings, channels, invites, requests, bans, safety, queue, log). */
export const Route = createFileRoute("/_app/g/$handle/manage/$section")({ component: SectionPage });

function SectionPage() {
  const { section } = Route.useParams();
  return <ManageSectionScreen key={section} section={section} />;
}
