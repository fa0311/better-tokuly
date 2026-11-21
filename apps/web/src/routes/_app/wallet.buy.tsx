import { createFileRoute } from "@tanstack/react-router";
import { BuyScreen } from "#/features/wallet/buy-screen";

export const Route = createFileRoute("/_app/wallet/buy")({ component: BuyScreen });
