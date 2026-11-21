import { createFileRoute } from "@tanstack/react-router";
import { WalletHomeScreen } from "#/features/wallet/wallet-home-screen";

export const Route = createFileRoute("/_app/wallet/")({ component: WalletHomeScreen });
