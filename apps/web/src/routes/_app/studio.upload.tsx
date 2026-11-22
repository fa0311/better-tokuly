import { createFileRoute } from "@tanstack/react-router";
import { UploadScreen } from "#/features/video/studio/upload-screen";

export const Route = createFileRoute("/_app/studio/upload")({ component: UploadScreen });
