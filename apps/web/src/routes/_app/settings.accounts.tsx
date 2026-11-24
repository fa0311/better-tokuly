import { createFileRoute } from "@tanstack/react-router";
import { AccountsSection } from "#/features/settings/accounts/accounts-section";

export const Route = createFileRoute("/_app/settings/accounts")({ component: AccountsSection });
