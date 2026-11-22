import { createFileRoute } from "@tanstack/react-router";
import { TokensScreen } from "#/features/dev/tokens";

export const Route = createFileRoute("/_app/developers/tokens")({ component: TokensScreen });
