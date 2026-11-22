import { createFileRoute } from "@tanstack/react-router";
import { PollsScreen } from "#/features/ask/polls-screen";

export const Route = createFileRoute("/_app/ask/polls")({ component: PollsScreen });
