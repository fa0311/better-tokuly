import type { common, notes } from "@tokuly/api";
import { Plus } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { useMe } from "#/stores/session";
import { useStoryTray, useUserStories } from "./api";
import { CreateStoryDialog } from "./create";
import { openStoryViewer, type ViewerReel } from "./viewer-store";

/** The ring around a face with stories: colourful while something is unseen, quiet once everything was watched. */
export function Ring({
  unseen,
  children,
  className,
}: {
  unseen: boolean;
  children: ReactNode;
  className?: string;
}) {
  return (
    <span
      className={cn(
        "inline-block rounded-[1.45rem] p-[3px]",
        unseen ? "bg-gradient-to-tr from-sun via-pink to-brand" : "bg-line",
        className,
      )}
    >
      <span className="block rounded-[1.25rem] bg-bg p-[2px]">{children}</span>
    </span>
  );
}

const toViewerReel = (reel: notes.StoryReel): ViewerReel => ({
  user: reel.user,
  stories: reel.stories,
});

/**
 * The story tray above the home timeline: the viewer's own reel first (with
 * the button that posts a story), then friends — those with something
 * unseen before the rest. A tap plays from that person on.
 */
export function StoryTray() {
  const { t } = useTranslation("stories");
  const me = useMe();
  const tray = useStoryTray();
  const [creating, setCreating] = useState(false);
  if (!me?.user) return null;

  const reels = tray.data ?? [];
  const own = reels.find((reel) => reel.user?.id === me.user!.id);
  const friends = reels.filter((reel) => reel.user?.id !== me.user!.id);
  const play = (reel: notes.StoryReel) =>
    openStoryViewer(reels.map(toViewerReel), { reelIndex: reels.indexOf(reel) });

  return (
    <section
      aria-label={t("tray.title")}
      className="-mx-3 mb-2.5 sm:-mx-4"
      data-testid="story-tray"
    >
      <ul className="scrollbar-none flex gap-2.5 overflow-x-auto px-3 pt-1 pb-1.5 sm:px-4">
        <li className="relative shrink-0">
          <button
            type="button"
            onClick={() => (own ? play(own) : setCreating(true))}
            aria-label={own ? t("tray.mine") : t("tray.add")}
            className="flex w-[4.75rem] flex-col items-center gap-1"
            data-testid="story-reel-own"
          >
            <Ring unseen={false} className={own ? undefined : "bg-transparent"}>
              <Avatar user={me.user} size="lg" />
            </Ring>
            <span className="w-full truncate text-center text-xs font-bold text-ink-soft">
              {own ? t("tray.mine") : t("tray.add")}
            </span>
          </button>
          <button
            type="button"
            aria-label={t("tray.add")}
            onClick={() => setCreating(true)}
            className="sticker-sm pressable absolute top-[2.9rem] right-0 grid size-7 place-items-center rounded-full bg-lime text-[#17122b]"
            data-testid="story-add"
          >
            <Plus className="size-4" strokeWidth={3.5} />
          </button>
        </li>
        {tray.isPending &&
          [0, 1, 2].map((index) => (
            <li
              key={index}
              className="flex w-[4.75rem] shrink-0 flex-col items-center gap-1.5 pt-1"
            >
              <Skeleton className="size-[4.25rem] rounded-[1.25rem]" />
              <Skeleton className="h-3 w-12 rounded-full" />
            </li>
          ))}
        {friends.map((reel) => (
          <li key={reel.user?.id} className="shrink-0">
            <button
              type="button"
              onClick={() => play(reel)}
              aria-label={t(reel.hasUnseen ? "tray.unseen" : "tray.seen", {
                name: reel.user?.displayName ?? "",
              })}
              className="flex w-[4.75rem] flex-col items-center gap-1"
              data-testid="story-reel"
              data-unseen={reel.hasUnseen}
            >
              <Ring unseen={reel.hasUnseen}>
                <Avatar user={reel.user} size="lg" />
              </Ring>
              <span
                className={cn(
                  "w-full truncate text-center text-xs",
                  reel.hasUnseen ? "font-black text-ink" : "font-bold text-ink-faint",
                )}
              >
                {reel.user?.displayName}
              </span>
            </button>
          </li>
        ))}
        {!tray.isPending && friends.length === 0 && (
          <li className="flex min-w-0 items-center px-2 pb-5 text-sm leading-relaxed font-bold text-ink-faint">
            {t("tray.empty")}
          </li>
        )}
      </ul>
      {creating && <CreateStoryDialog onClose={() => setCreating(false)} />}
    </section>
  );
}

/**
 * Wraps a person's face (on their profile): while they have stories the
 * viewer may see, the face gets the story ring and plays them on tap.
 */
export function StoryRing({ user, children }: { user: common.UserSummary; children: ReactNode }) {
  const { t } = useTranslation("stories");
  const stories = useUserStories(user.id);
  const live = stories.data ?? [];
  if (live.length === 0) return <>{children}</>;
  return (
    <button
      type="button"
      aria-label={t("tray.play", { name: user.displayName })}
      onClick={() => openStoryViewer([{ user, stories: live }])}
      className="pressable rounded-[2.2rem]"
      data-testid="story-ring"
    >
      <span
        className={cn(
          "block rounded-[2.35rem] p-1",
          live.some((story) => !story.viewed)
            ? "bg-gradient-to-tr from-sun via-pink to-brand"
            : "bg-line",
        )}
      >
        <span className="block rounded-[2.15rem] bg-bg p-[3px]">{children}</span>
      </span>
    </button>
  );
}
