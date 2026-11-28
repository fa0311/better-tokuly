import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { type live, toMillis } from "@tokuly/api";
import { Play, Scissors } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { TextField } from "#/components/ui/field";
import { EmptyState, Page } from "#/components/ui/page";
import { PaneSpinner } from "#/components/ui/spinner";
import { type PlayerHandle, VideoPlayer } from "#/features/video";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { clipClient, liveKeys, useStream } from "../api";
import { useArchiveVideo } from "../archive";
import { CLIP_MAX_SECONDS, clampClipRange } from "../model";
import { clock } from "../player/live-player";

/** How far back from "now" the timeline of a stream that is on air reaches. */
const LIVE_WINDOW_SECONDS = 180;

/**
 * The range picker of a clip: a bar showing the chosen stretch of the
 * source, and a slider for each of its ends. The range never exceeds a
 * minute; the end being moved pulls the other along.
 */
function RangeTimeline({
  min,
  max,
  start,
  end,
  onChange,
}: {
  min: number;
  max: number;
  start: number;
  end: number;
  onChange(start: number, end: number): void;
}) {
  const { t } = useTranslation("live");
  const span = Math.max(0.1, max - min);
  const move = (which: "start" | "end", value: number) => {
    const next = clampClipRange(
      (which === "start" ? value : start) - min,
      (which === "end" ? value : end) - min,
      span,
      which,
    );
    onChange(next.start + min, next.end + min);
  };
  return (
    <div data-testid="clip-timeline">
      <div className="relative h-10 overflow-hidden rounded-xl border-2 border-ink bg-surface-3">
        <div
          className="absolute inset-y-0 bg-lime"
          style={{
            left: `${((start - min) / span) * 100}%`,
            width: `${((end - start) / span) * 100}%`,
          }}
        />
        <span className="absolute inset-0 grid place-items-center text-sm font-black text-[#17122b] tabular-nums">
          {t("clip.length", { seconds: (end - start).toFixed(1) })}
        </span>
      </div>
      <div className="mt-3 grid gap-3 sm:grid-cols-2">
        {(["start", "end"] as const).map((which) => {
          const value = which === "start" ? start : end;
          return (
            <label key={which} className="flex flex-col gap-1 text-sm font-bold text-ink-soft">
              <span className="flex justify-between px-1">
                {t(which === "start" ? "clip.start" : "clip.end")}
                <span className="text-ink tabular-nums">{clock(value)}</span>
              </span>
              <input
                type="range"
                min={min}
                max={max}
                step={0.1}
                value={value}
                onChange={(event) => move(which, Number(event.target.value))}
                className="h-2 w-full cursor-pointer accent-[var(--brand)]"
                aria-valuetext={clock(value)}
              />
            </label>
          );
        })}
      </div>
    </div>
  );
}

function ClipForm({
  defaultTitle,
  duration,
  pending,
  onSubmit,
}: {
  defaultTitle: string;
  duration: number;
  pending: boolean;
  onSubmit(title: string): void;
}) {
  const { t } = useTranslation("live");
  const [title, setTitle] = useState(defaultTitle.slice(0, 64));
  return (
    <form
      className="mt-4 flex flex-col gap-3"
      onSubmit={(event) => {
        event.preventDefault();
        if (title.trim()) onSubmit(title.trim());
      }}
    >
      <TextField
        label={t("clip.titleLabel")}
        value={title}
        maxLength={64}
        onChange={(event) => setTitle(event.target.value)}
        hint={t("clip.titleHint", { max: CLIP_MAX_SECONDS })}
      />
      <Button
        type="submit"
        size="lg"
        loading={pending}
        disabled={!title.trim() || duration <= 0}
        icon={<Scissors className="size-5" />}
      >
        {t("clip.publish")}
      </Button>
    </form>
  );
}

function useCreateClip() {
  const { t } = useTranslation("live");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: {
      streamId: string;
      videoId: string;
      title: string;
      startSeconds: number;
      durationSeconds: number;
    }) => clipClient().createClip(request),
    onSuccess: ({ clip }) => {
      void queryClient.invalidateQueries({ queryKey: liveKeys.clips });
      toast.success(t("clip.created"));
      if (clip) {
        queryClient.setQueryData(liveKeys.clip(clip.shortId), clip);
        void navigate({ to: "/clip/$clipId", params: { clipId: clip.shortId } });
      }
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "live" })),
  });
}

/** Cutting a clip out of a video (an archive included): play it, mark where the clip starts and ends. */
function VideoClipEditor({ videoId }: { videoId: string }) {
  const { t } = useTranslation("live");
  const query = useArchiveVideo(videoId);
  const player = useRef<PlayerHandle>(null);
  const create = useCreateClip();
  const duration = query.data?.playback?.durationSeconds ?? 0;
  const [range, setRange] = useState<{ start: number; end: number } | null>(null);
  const start = range?.start ?? 0;
  const end = range?.end ?? Math.min(duration, 30);
  const [previewing, setPreviewing] = useState(false);

  // Previewing plays the chosen stretch and stops at its end.
  useEffect(() => {
    if (!previewing) return;
    const timer = setInterval(() => {
      const handle = player.current;
      if (!handle) return;
      if (handle.currentTime() >= end || handle.paused()) {
        handle.pause();
        setPreviewing(false);
      }
    }, 100);
    return () => clearInterval(timer);
  }, [previewing, end]);

  if (query.isPending) return <PaneSpinner />;
  const video = query.data;
  if (!video?.playback)
    return (
      <EmptyState emoji="🎞️" title={t("clip.sourceGoneTitle")} body={t("clip.sourceGoneBody")} />
    );
  const mark = (which: "start" | "end") => {
    const at = player.current?.currentTime() ?? 0;
    setRange(
      clampClipRange(which === "start" ? at : start, which === "end" ? at : end, duration, which),
    );
  };
  return (
    <>
      <VideoPlayer ref={player} source={video.playback} title={video.summary?.title} />
      <div className="mt-4 flex flex-wrap gap-2">
        <Button variant="secondary" size="sm" onClick={() => mark("start")}>
          {t("clip.markStart")}
        </Button>
        <Button variant="secondary" size="sm" onClick={() => mark("end")}>
          {t("clip.markEnd")}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          icon={<Play className="size-4" />}
          onClick={() => {
            player.current?.seek(start);
            void player.current?.play();
            setPreviewing(true);
          }}
        >
          {t("clip.preview")}
        </Button>
      </div>
      <div className="mt-4">
        <RangeTimeline
          min={0}
          max={duration}
          start={start}
          end={end}
          onChange={(nextStart, nextEnd) => {
            setRange({ start: nextStart, end: nextEnd });
            player.current?.seek(nextStart !== start ? nextStart : nextEnd);
          }}
        />
      </div>
      <ClipForm
        defaultTitle={video.summary?.title ?? ""}
        duration={end - start}
        pending={create.isPending}
        onSubmit={(title) =>
          create.mutate({
            streamId: "",
            videoId,
            title,
            startSeconds: start,
            durationSeconds: Math.round((end - start) * 10) / 10,
          })
        }
      />
    </>
  );
}

/** Cutting a clip out of a stream that is on air: the last minutes up to the moment the viewer pressed "clip". */
function StreamClipEditor({ stream, at }: { stream: live.Stream; at: number }) {
  const { t } = useTranslation("live");
  const create = useCreateClip();
  const summary = stream.summary;
  const startedAtMs = summary?.startedAt ? toMillis(summary.startedAt) : Date.now();
  // The moment the editor opened is the end of what can be cut; the stream going on meanwhile changes nothing.
  const [windowEnd] = useState(() => Math.max(1, at > 0 ? at : (Date.now() - startedAtMs) / 1000));
  const windowStart = Math.max(0, windowEnd - LIVE_WINDOW_SECONDS);
  const [range, setRange] = useState(() => ({
    start: Math.max(windowStart, windowEnd - 30),
    end: windowEnd,
  }));
  /** The stretch whose recording is loaded into the preview. */
  const [previewed, setPreviewed] = useState(range);
  const playback = stream.playback;

  if (!summary || !playback?.dvrGetUrl)
    return <EmptyState emoji="📼" title={t("clip.noDvrTitle")} body={t("clip.noDvrBody")} />;
  const previewUrl = `${playback.dvrGetUrl}&start=${encodeURIComponent(
    new Date(startedAtMs + previewed.start * 1000).toISOString(),
  )}&duration=${Math.max(0.1, previewed.end - previewed.start).toFixed(1)}`;
  return (
    <>
      <video
        key={previewUrl}
        src={previewUrl}
        controls
        playsInline
        className="aspect-video w-full rounded-2xl bg-black"
        data-testid="clip-preview"
      />
      <div className="mt-4">
        <RangeTimeline
          min={windowStart}
          max={windowEnd}
          start={range.start}
          end={range.end}
          onChange={(start, end) => setRange({ start, end })}
        />
      </div>
      <Button
        className="mt-3"
        variant="secondary"
        size="sm"
        icon={<Play className="size-4" />}
        disabled={previewed.start === range.start && previewed.end === range.end}
        onClick={() => setPreviewed(range)}
      >
        {t("clip.previewRange")}
      </Button>
      <ClipForm
        defaultTitle={summary.title}
        duration={range.end - range.start}
        pending={create.isPending}
        onSubmit={(title) =>
          create.mutate({
            streamId: summary.id,
            videoId: "",
            title,
            startSeconds: range.start,
            durationSeconds: Math.round((range.end - range.start) * 10) / 10,
          })
        }
      />
    </>
  );
}

/** `/live/clips/new?stream=<id>&at=<seconds>` or `?video=<id>`: cut a clip and publish it. */
export function ClipEditorScreen({
  streamId,
  videoId,
  at,
}: {
  streamId: string;
  videoId: string;
  at: number;
}) {
  const { t } = useTranslation("live");
  const stream = useStream(videoId ? "" : streamId);
  return (
    <Page title={t("clip.editorTitle")} back={streamId ? `/live/${streamId}` : "/live"}>
      {videoId ? (
        <VideoClipEditor videoId={videoId} />
      ) : !streamId ? (
        <EmptyState emoji="✂️" title={t("clip.noSourceTitle")} body={t("clip.noSourceBody")} />
      ) : stream.isPending ? (
        <PaneSpinner />
      ) : stream.data ? (
        <StreamClipEditor stream={stream.data} at={at} />
      ) : (
        <EmptyState emoji="📡" title={t("notFound.title")} body={t("notFound.body")} />
      )}
    </Page>
  );
}
