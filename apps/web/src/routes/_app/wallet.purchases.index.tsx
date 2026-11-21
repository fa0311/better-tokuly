import { createFileRoute } from "@tanstack/react-router";
import { PurchasesScreen } from "#/features/wallet/purchases-screen";

export const Route = createFileRoute("/_app/wallet/purchases/")({ component: PurchasesScreen });
