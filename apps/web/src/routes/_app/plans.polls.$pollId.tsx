import { createFileRoute } from "@tanstack/react-router";
import { PollScreen } from "#/features/plans/poll/poll-screen";

// `/plans/polls/<short id>`: the link of a date poll (the poll's ID works too).
export const Route = createFileRoute("/_app/plans/polls/$pollId")({ component: PollPage });

function PollPage() {
  const { pollId } = Route.useParams();
  return <PollScreen key={pollId} pollId={pollId} />;
}
