import { createFileRoute } from "@tanstack/react-router";
import { FlamesScreen } from "#/features/ask/flames-screen";

/** The flames inbox (`ask.flame` notifications lead here). */
export const Route = createFileRoute("/_app/ask/flames")({ component: FlamesScreen });
