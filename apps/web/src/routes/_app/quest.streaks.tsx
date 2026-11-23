import { createFileRoute } from "@tanstack/react-router";
import { StreaksScreen } from "#/features/quest/streaks-screen";

// `/quest/streaks`: where a profile's streak card and the streak notifications lead.
export const Route = createFileRoute("/_app/quest/streaks")({ component: StreaksScreen });
