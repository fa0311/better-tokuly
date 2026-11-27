import { createFileRoute } from "@tanstack/react-router";
import { SpendingLimitScreen } from "#/features/wallet/spending-limit";

export const Route = createFileRoute("/_app/wallet/limit")({ component: SpendingLimitScreen });
