import { createFileRoute } from "@tanstack/react-router";
import { SentScreen } from "#/features/ask/sent-screen";

export const Route = createFileRoute("/_app/ask/sent")({ component: SentScreen });
