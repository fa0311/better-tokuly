import { createFileRoute } from "@tanstack/react-router";
import { AccountSection } from "#/features/settings/account/account-section";

export const Route = createFileRoute("/_app/settings/account")({ component: AccountSection });
