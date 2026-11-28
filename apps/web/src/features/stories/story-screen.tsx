import { Link, useNavigate, useRouter } from "@tanstack/react-router";
import { isNotFound } from "@tokuly/api";
import { useEffect, useRef } from "react";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { useStory } from "./api";
import { closeStoryViewer, openStoryViewer, useStoryViewer } from "./viewer-store";

/**
 * `/stories/<id>`: where notifications, the feed and reports point for a
 * story. It plays the story full screen and, once the viewer is closed,
 * returns to where the person came from.
 */
export function StoryScreen({ storyId }: { storyId: string }) {
  const { t } = useTranslation("stories");
  const router = useRouter();
  const navigate = useNavigate();
  const story = useStory(storyId);
  const opened = useRef(false);
  const loaded = story.data;

  useEffect(() => {
    if (!loaded || opened.current) return;
    opened.current = true;
    openStoryViewer([{ user: loaded.owner, stories: [loaded] }], {
      storyId: loaded.id,
      onClose: () =>
        router.history.canGoBack() ? router.history.back() : void navigate({ to: "/notes" }),
    });
  }, [loaded, router, navigate]);

  // Leaving the page by other means (a link inside the viewer) closes the viewer without navigating again.
  useEffect(
    () => () => {
      if (useStoryViewer.getState().reels) {
        useStoryViewer.setState({ onClose: undefined });
        closeStoryViewer();
      }
    },
    [],
  );

  if (story.isPending || loaded) {
    return (
      <Page title={t("page.title")} back="/notes">
        <Skeleton className="mx-auto aspect-[9/16] max-h-[70dvh] rounded-[1.75rem]" />
      </Page>
    );
  }
  const gone = isNotFound(story.error);
  return (
    <Page title={t("page.title")} back="/notes">
      <EmptyState
        emoji={gone ? "🫧" : "🌧️"}
        title={gone ? t("page.gone") : errorMessage(story.error, { namespace: "stories" })}
        body={gone ? t("page.goneBody") : undefined}
        action={
          <Link
            to="/notes"
            className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand"
          >
            {t("page.toNotes")}
          </Link>
        }
      />
    </Page>
  );
}
