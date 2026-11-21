import { createFileRoute } from "@tanstack/react-router";
import { PrivacySection } from "#/features/settings/privacy/privacy-section";

export const Route = createFileRoute("/_app/settings/privacy/")({ component: PrivacySection });
