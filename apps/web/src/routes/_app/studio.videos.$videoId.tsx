import { createFileRoute } from "@tanstack/react-router";
import { EDITOR_TABS, type EditorTab, VideoEditorScreen } from "#/features/video/studio/video-editor";

/** `/studio/videos/<short id>?tab=subtitles|analytics`: where "processing finished" notifications lead. */
export const Route = createFileRoute("/_app/studio/videos/$videoId")({
  validateSearch: (search: Record<string, unknown>): { tab?: Exclude<EditorTab, "details"> } => {
    const tab = EDITOR_TABS.find((candidate) => candidate === search.tab);
    return tab && tab !== "details" ? { tab } : {};
  },
  component: EditorPage,
});

function EditorPage() {
  const { videoId } = Route.useParams();
  const { tab = "details" } = Route.useSearch();
  const navigate = Route.useNavigate();
  return <VideoEditorScreen key={videoId} videoId={videoId} tab={tab} onTabChange={(next) => navigate({ search: next === "details" ? {} : { tab: next }, replace: true })} />;
}
