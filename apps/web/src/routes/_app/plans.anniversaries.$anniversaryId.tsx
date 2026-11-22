import { createFileRoute } from "@tanstack/react-router";
import { AnniversaryScreen } from "#/features/plans/countdown/anniversary-screen";

export const Route = createFileRoute("/_app/plans/anniversaries/$anniversaryId")({
  component: AnniversaryPage,
});

function AnniversaryPage() {
  const { anniversaryId } = Route.useParams();
  return <AnniversaryScreen anniversaryId={anniversaryId} />;
}
