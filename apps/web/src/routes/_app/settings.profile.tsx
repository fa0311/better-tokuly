import { createFileRoute } from "@tanstack/react-router";
import { ProfileSection } from "#/features/settings/profile/profile-section";

export const Route = createFileRoute("/_app/settings/profile")({ component: ProfileSection });
