import { createFileRoute } from "@tanstack/react-router";
import { PollNewScreen } from "#/features/plans/poll/poll-new-screen";

export const Route = createFileRoute("/_app/plans/polls/new")({ component: PollNewScreen });
