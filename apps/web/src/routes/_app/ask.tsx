import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout of the question-box area (質問箱・お題・ほめ投票): the hub (index) and the `ask.*.tsx` pages.
export const Route = createFileRoute("/_app/ask")({ component: Outlet });
