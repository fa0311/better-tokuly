import { createFileRoute } from "@tanstack/react-router";
import { ClipEditorScreen } from "#/features/live/clips/clip-editor";

interface ClipSearch {
  /** A stream that is on air (UUID or short ID) … */
  stream?: string;
  /** … or a video, an archive included. */
  video?: string;
  /** Where on the stream's timeline the viewer was, in seconds. */
  at?: number;
}

// `/live/clips/new?stream=<id>&at=<seconds>` or `?video=<id>`: cut a clip.
export const Route = createFileRoute("/_app/live/clips/new")({
  validateSearch: (search: Record<string, unknown>): ClipSearch => ({
    ...(typeof search.stream === "string" && search.stream ? { stream: search.stream } : {}),
    ...(search.video !== undefined && search.video !== "" ? { video: String(search.video) } : {}),
    ...(Number.isFinite(Number(search.at)) && Number(search.at) > 0
      ? { at: Number(search.at) }
      : {}),
  }),
  component: ClipEditorPage,
});

function ClipEditorPage() {
  const { stream, video, at } = Route.useSearch();
  return (
    <ClipEditorScreen
      key={`${stream}:${video}`}
      streamId={stream ?? ""}
      videoId={video ?? ""}
      at={at ?? 0}
    />
  );
}
