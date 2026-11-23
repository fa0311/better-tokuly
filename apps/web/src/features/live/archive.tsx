import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { errorReason, live } from "@tokuly/api";
import { Film, Hourglass, Scissors, TriangleAlert } from "lucide-react";
import { type RefObject, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { EmptyState } from "#/components/ui/page";
import { type PlayerHandle, VideoPlayer } from "#/features/video";
import { cn } from "#/lib/cn";
import { chatClient, liveKeys, useGiftTiers, videoClient } from "./api";
import { ChatList } from "./chat/chat-list";
import { isOpenVisibility } from "./labels";
import { addChatMessages } from "./model";

const { ArchiveStatus } = live;

/** The archive video of a stream as the caller may see it; `null` when it is gone or not theirs to watch. */
export function useArchiveVideo(videoId: string) {
  return useQuery({
    queryKey: liveKeys.archive(videoId),
    queryFn: async () => {
      try {
        return (await videoClient().getVideo({ videoId })).video ?? null;
      } catch (error) {
        if (errorReason(error) === "VIDEO_NOT_FOUND") return null;
        throw error;
      }
    },
    enabled: videoId !== "",
    // While the recording is still being transcoded there is nothing to play yet.
    refetchInterval: (query) => (query.state.data && !query.state.data.playback ? 8000 : false),
  });
}

/** The chat of an ended stream is fetched in windows of this length as playback moves through it. */
const WINDOW_MS = 5 * 60_000;

/**
 * Chat replay: the lines of the stream's chat appear as the archive reaches
 * the moment they were written, and disappear again when it is rewound.
 */
export function ChatReplay({
  streamId,
  player,
  className,
}: {
  streamId: string;
  player: RefObject<PlayerHandle | null>;
  className?: string;
}) {
  const { t } = useTranslation("live");
  const tiers = useGiftTiers().data?.tiers ?? [];
  const [positionMs, setPositionMs] = useState(0);
  const [loaded, setLoaded] = useState<live.ChatMessage[]>([]);
  /** Windows already asked for, by their index. */
  const fetched = useRef(new Set<number>());

  useEffect(() => {
    const timer = setInterval(() => {
      const seconds = player.current?.currentTime() ?? 0;
      setPositionMs(Math.floor(seconds * 1000));
    }, 500);
    return () => clearInterval(timer);
  }, [player]);

  const windowIndex = Math.floor(positionMs / WINDOW_MS);
  useEffect(() => {
    let cancelled = false;
    // The window being played and the one after it, so lines are there before they are due.
    for (const index of [windowIndex, windowIndex + 1]) {
      if (fetched.current.has(index)) continue;
      fetched.current.add(index);
      const load = async (fromOffsetMs: number) => {
        const response = await chatClient().listChatReplay({
          streamId,
          fromOffsetMs: BigInt(fromOffsetMs),
          toOffsetMs: BigInt((index + 1) * WINDOW_MS),
        });
        if (cancelled) return;
        setLoaded((current) => addChatMessages(current, response.messages, 5000));
        const last = response.messages.at(-1);
        // A busy stretch comes in several answers.
        if (response.truncated && last && Number(last.offsetMs) + 1 < (index + 1) * WINDOW_MS)
          await load(Number(last.offsetMs) + 1);
      };
      load(index * WINDOW_MS).catch(() => fetched.current.delete(index));
    }
    return () => {
      cancelled = true;
    };
  }, [streamId, windowIndex]);

  const shown = loaded.filter((message) => Number(message.offsetMs) <= positionMs).slice(-300);
  return (
    <section
      className={cn("flex min-h-0 flex-col bg-surface", className)}
      aria-label={t("replay.title")}
      data-testid="chat-replay"
    >
      <header className="flex h-12 shrink-0 items-center border-b-2 border-line px-4">
        <h2 className="font-black">{t("replay.title")}</h2>
      </header>
      <ChatList
        className="flex-1"
        messages={shown}
        tiers={tiers}
        empty={<EmptyState emoji="🎞️" title={t("replay.emptyTitle")} body={t("replay.emptyBody")} />}
      />
    </section>
  );
}

/**
 * What an ended stream shows where the player was: its archive once that
 * exists (with a way to cut clips from it), or where the recording stands.
 */
export function ArchiveStage({
  stream,
  player,
}: {
  stream: live.Stream;
  player: RefObject<PlayerHandle | null>;
}) {
  const { t } = useTranslation("live");
  const archive = useArchiveVideo(stream.archiveVideoId);
  const video = archive.data;
  const status = stream.archiveStatus;

  if (video?.playback) {
    return (
      <div data-testid="archive-player">
        <VideoPlayer
          ref={player}
          source={video.playback}
          title={stream.summary?.title}
          shortcuts
          className="md:rounded-2xl"
        />
        <div className="flex flex-wrap items-center gap-2 px-3 pt-2 md:px-0">
          <Link
            to="/watch/$videoId"
            params={{ videoId: stream.archiveVideoId }}
            className="pressable sticker-sm inline-flex h-9 items-center gap-1.5 rounded-full bg-surface px-4 text-sm font-black"
            data-testid="archive-link"
          >
            <Film className="size-4" />
            {t("archive.open")}
          </Link>
          {stream.summary && isOpenVisibility(stream.summary.visibility) && (
            <Link
              to="/live/clips/new"
              search={{ video: stream.archiveVideoId }}
              className="pressable sticker-sm inline-flex h-9 items-center gap-1.5 rounded-full bg-surface px-4 text-sm font-black"
            >
              <Scissors className="size-4" />
              {t("clip.make")}
            </Link>
          )}
        </div>
      </div>
    );
  }

  const waiting =
    status === ArchiveStatus.PENDING ||
    status === ArchiveStatus.PROCESSING ||
    (status === ArchiveStatus.READY && video !== null);
  return (
    <div className="grid aspect-video w-full place-items-center bg-[#17122b] p-6 text-center text-white md:rounded-2xl">
      <div className="flex max-w-sm flex-col items-center gap-2">
        <p className="text-4xl" aria-hidden>
          {waiting ? "📼" : "👋"}
        </p>
        <p className="text-xl font-black">{t("ended.title")}</p>
        {waiting ? (
          <p
            className="flex items-center gap-1.5 text-sm font-bold text-white/80"
            data-testid="archive-pending"
          >
            <Hourglass className="size-4" />
            {t("archive.processing")}
          </p>
        ) : status === ArchiveStatus.FAILED ? (
          <p className="flex items-center gap-1.5 text-sm font-bold text-white/80">
            <TriangleAlert className="size-4" />
            {t("archive.failed")}
          </p>
        ) : (
          <p className="text-sm font-bold text-white/80">{t("ended.noArchive")}</p>
        )}
        {status === ArchiveStatus.READY && stream.archiveVideoId && (
          <Link
            to="/watch/$videoId"
            params={{ videoId: stream.archiveVideoId }}
            className="pressable mt-2 inline-flex h-10 items-center gap-1.5 rounded-full bg-white px-4 text-sm font-black text-[#17122b]"
            data-testid="archive-link"
          >
            <Film className="size-4" />
            {t("archive.open")}
          </Link>
        )}
      </div>
    </div>
  );
}
