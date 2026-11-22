import { createFileRoute } from "@tanstack/react-router";
import { NewChannelScreen } from "#/features/video/studio/channel-form";

export const Route = createFileRoute("/_app/studio/channels/new")({ component: NewChannelScreen });
