import { createFileRoute } from "@tanstack/react-router";
import { EditSubmissionScreen } from "#/features/emoji/submit-screen";

export const Route = createFileRoute("/_app/emoji/submissions/$emojiId")({ component: EditPage });

function EditPage() {
  const { emojiId } = Route.useParams();
  return <EditSubmissionScreen key={emojiId} emojiId={emojiId} />;
}
