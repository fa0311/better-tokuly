import { createFileRoute } from "@tanstack/react-router";
import { EarningsScreen } from "#/features/wallet/earnings-screen";

export const Route = createFileRoute("/_app/wallet/earnings")({ component: EarningsScreen });
