import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { common, fromDate, toMillis, video } from "@tokuly/api";
import { CalendarClock, Eye, EyeOff, Play, Rocket, Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { waitForAsset } from "#/lib/upload";
import {
  patchVideoEverywhere,
  removeVideoEverywhere,
  useVideo,
  videoClient,
  videoKeys,
} from "../api";
import { CircleField, LockedNote } from "../audience";
import { toLocalInput } from "../format";
import { VIDEO_VISIBILITIES } from "../labels";
import { type PlayerHandle, VideoPlayer } from "../player/player";
import { type RangeDays, RangePicker, VideoAnalytics } from "./analytics";
import { toChapters } from "./chapters";
import {
  type DetailsDraft,
  DetailsForm,
  draftFrom,
  draftProblem,
  draftSignature,
} from "./details-form";
import { FormSection, VideoStateChips } from "./shared";
import { SubtitlesTab } from "./subtitles";
import { ThumbnailField } from "./thumbnail-field";

const { VideoStatus, ProcessingState } = video;

export const EDITOR_TABS = ["details", "subtitles", "analytics"] as const;
export type EditorTab = (typeof EDITOR_TABS)[number];

/**
 * `/studio/videos/<id>`: one video of the creator. The details, subtitles
 * and analytics are tabs; the card at the top always says where the video
 * stands — still processing, a draft, scheduled, published — and offers the
 * next step.
 */
export function VideoEditorScreen({
  videoId,
  tab,
  onTabChange,
}: {
  videoId: string;
  tab: EditorTab;
  onTabChange(tab: EditorTab): void;
}) {
  const { t } = useTranslation("video");
  const query = useVideo(videoId);
  const item = query.data;

  if (query.isPending) {
    return (
      <Page title={t("studio.editor.pageTitle")} back="/studio" wide>
        <Skeleton className="h-40 rounded-card" />
        <Skeleton className="mt-4 h-96 rounded-card" />
      </Page>
    );
  }
  if (query.isError || !item?.summary || !item.viewer?.isOwner) {
    return (
      <Page title={t("studio.editor.pageTitle")} back="/studio">
        <EmptyState
          emoji="🫥"
          title={t("studio.editor.notFound")}
          body={t("studio.editor.notFoundBody")}
        />
      </Page>
    );
  }
  return (
    <Editor
      key={item.summary.id}
      item={item}
      videoKey={videoId}
      tab={tab}
      onTabChange={onTabChange}
    />
  );
}

function Editor({
  item,
  videoKey,
  tab,
  onTabChange,
}: {
  item: video.Video;
  videoKey: string;
  tab: EditorTab;
  onTabChange(tab: EditorTab): void;
}) {
  const { t } = useTranslation("video");
  const queryClient = useQueryClient();
  const player = useRef<PlayerHandle>(null);
  const summary = item.summary as video.VideoSummary;
  const [draft, setDraft] = useState<DetailsDraft>(() => draftFrom(item));
  // What the server holds, in the draft's shape: the form is "changed" while it differs.
  const [saved, setSaved] = useState<DetailsDraft>(() => draftFrom(item));
  const [days, setDays] = useState<RangeDays>(28);
  const dirty = draftSignature(draft) !== draftSignature(saved);
  const problem = draftProblem(draft, summary.durationSeconds);

  const store = (updated: video.Video | undefined) => {
    if (!updated) return;
    queryClient.setQueryData(videoKeys.video(videoKey), updated);
    if (updated.summary)
      patchVideoEverywhere(queryClient, updated.summary.id, () => updated.summary ?? null);
    void queryClient.invalidateQueries({ queryKey: videoKeys.studio });
  };

  const save = useMutation({
    mutationFn: async (next: DetailsDraft) =>
      (
        await videoClient().updateVideo({
          videoId: summary.id,
          title: next.title.trim(),
          description: next.description,
          tags: { tags: next.tags },
          category: next.category,
          game: next.game.trim(),
          visibility: next.visibility,
          circleId: next.visibility === common.Visibility.CIRCLE ? next.circleId : "",
          ageRestricted: next.ageRestricted,
          commentsEnabled: next.commentsEnabled,
          notifySubscribers: next.notifySubscribers,
          chapters: { chapters: toChapters(next.chapters) },
          allowLink: next.visibility === common.Visibility.FRIENDS && next.allowLink,
        })
      ).video,
    onSuccess: (updated, next) => {
      store(updated);
      setSaved(next);
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  /** Publishing can change the audience: the form follows, so a later save does not undo it. */
  const storePublished = (updated: video.Video | undefined) => {
    store(updated);
    const visibility = updated?.summary?.visibility;
    if (!visibility) return;
    const circleId = updated?.studio?.circleId ?? "";
    setDraft((current) => ({ ...current, visibility, circleId }));
    setSaved((current) => ({ ...current, visibility, circleId }));
  };

  return (
    <Page title={summary.title || t("studio.editor.pageTitle")} back="/studio" wide>
      <div className="grid gap-5 lg:grid-cols-[minmax(0,1fr)_22rem]">
        <aside className="flex flex-col gap-4 lg:col-start-2 lg:row-start-1">
          <StatusCard
            item={item}
            videoKey={videoKey}
            draft={draft}
            dirty={dirty}
            problem={problem}
            saveDraft={() => save.mutateAsync(draft)}
            onStored={storePublished}
          />
          {item.playback ? (
            <VideoPlayer
              ref={player}
              source={item.playback}
              chapters={item.chapters}
              subtitles={item.subtitles}
              className="sticker rounded-card"
            />
          ) : (
            <ProcessingCard item={item} videoKey={videoKey} />
          )}
          <FormSection title={t("studio.thumbnail.title")} lead={t("studio.thumbnail.lead")}>
            <ThumbnailField video={item} videoKey={videoKey} />
          </FormSection>
        </aside>

        <div className="min-w-0 lg:col-start-1 lg:row-start-1">
          <Tabs<EditorTab>
            className="mb-4"
            value={tab}
            onChange={onTabChange}
            items={[
              { value: "details", label: t("studio.editor.tabs.details") },
              { value: "subtitles", label: t("studio.editor.tabs.subtitles") },
              { value: "analytics", label: t("studio.editor.tabs.analytics") },
            ]}
          />
          {tab === "details" && (
            <>
              <DetailsForm
                draft={draft}
                onChange={setDraft}
                durationSeconds={summary.durationSeconds}
                playerTime={item.playback ? () => player.current?.currentTime() ?? 0 : undefined}
              />
              <div
                className={cn(
                  "sticky bottom-24 z-10 mt-5 flex items-center justify-end gap-3 md:bottom-4",
                  !dirty && "pointer-events-none opacity-0",
                )}
                aria-hidden={!dirty}
              >
                <p className="sticker-sm rounded-full bg-surface px-4 py-2 text-sm font-bold">
                  {problem ? t(`studio.editor.problem.${problem}`) : t("studio.editor.unsaved")}
                </p>
                <Button
                  size="lg"
                  className="shadow-pop"
                  disabled={!dirty || problem !== null}
                  loading={save.isPending}
                  tabIndex={dirty ? 0 : -1}
                  onClick={() =>
                    save.mutate(draft, { onSuccess: () => toast.success(t("studio.toast.saved")) })
                  }
                >
                  {t("actions.save")}
                </Button>
              </div>
            </>
          )}
          {tab === "subtitles" && <SubtitlesTab videoId={summary.id} />}
          {tab === "analytics" && (
            <div className="flex flex-col gap-4">
              <RangePicker days={days} onChange={setDays} />
              <VideoAnalytics
                videoId={summary.id}
                durationSeconds={summary.durationSeconds}
                days={days}
              />
            </div>
          )}
        </div>
      </div>
    </Page>
  );
}

/** While the upload is being processed: how far it is, live, and what happens next. */
function ProcessingCard({ item, videoKey }: { item: video.Video; videoKey: string }) {
  const { t } = useTranslation("video");
  const queryClient = useQueryClient();
  const [progress, setProgress] = useState(0);
  const assetId = item.studio?.assetId;
  const processing = item.summary?.processing === ProcessingState.PROCESSING;

  useEffect(() => {
    if (!processing || !assetId) return;
    const abort = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    // The video follows its upload a moment later (and says so over realtime); ask again in case that push was missed.
    const reread = (attempt = 0) => {
      if (abort.signal.aborted) return;
      void queryClient.invalidateQueries({ queryKey: videoKeys.video(videoKey) });
      if (attempt < 4) timer = setTimeout(() => reread(attempt + 1), 1500 * (attempt + 1));
    };
    waitForAsset(assetId, (asset) => setProgress(asset.progress), abort.signal).then(
      () => reread(),
      () => reread(),
    );
    return () => {
      abort.abort();
      clearTimeout(timer);
    };
  }, [processing, assetId, queryClient, videoKey]);

  const failed = item.summary?.processing === ProcessingState.FAILED;
  return (
    <section
      className="sticker rounded-card bg-surface p-4"
      data-testid="processing-card"
      aria-live="polite"
    >
      <p className="text-3xl" aria-hidden>
        {failed ? "😢" : "⏳"}
      </p>
      <h2 className="mt-1 font-black">
        {failed ? t("studio.processing.failedTitle") : t("studio.processing.title")}
      </h2>
      <p className="mt-0.5 text-sm leading-relaxed text-ink-soft">
        {failed
          ? t(`studio.processing.reasons.${item.studio?.processingError || "PROCESSING_FAILED"}`, {
              defaultValue: t("studio.processing.reasons.PROCESSING_FAILED"),
            })
          : t("studio.processing.lead")}
      </p>
      {!failed && (
        <div
          className="mt-3 h-3 overflow-hidden rounded-full border-2 border-ink bg-surface-3"
          role="progressbar"
          aria-label={t("studio.processing.title")}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={progress}
        >
          <div
            className="h-full bg-lime transition-[width] duration-300"
            style={{ width: `${Math.max(4, progress)}%` }}
          />
        </div>
      )}
    </section>
  );
}

/** Where the video stands and the step that comes next: publish, schedule, take back, delete. */
function StatusCard({
  item,
  videoKey,
  draft,
  dirty,
  problem,
  saveDraft,
  onStored,
}: {
  item: video.Video;
  videoKey: string;
  draft: DetailsDraft;
  dirty: boolean;
  problem: string | null;
  saveDraft(): Promise<unknown>;
  onStored(updated: video.Video | undefined): void;
}) {
  const { t } = useTranslation("video");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const summary = item.summary as video.VideoSummary;
  // The dialog starts afresh every time it is opened: `opened` counts the openings and keys it.
  const [publishing, setPublishing] = useState(false);
  const [opened, setOpened] = useState(0);
  const openPublish = () => {
    setOpened((count) => count + 1);
    setPublishing(true);
  };
  const [deleting, setDeleting] = useState(false);
  const failed = summary.processing === ProcessingState.FAILED;
  const onError = (error: unknown) => toast.error(errorMessage(error));

  const unpublish = useMutation({
    mutationFn: () => videoClient().unpublishVideo({ videoId: summary.id }),
    onSuccess: ({ video: updated }) => {
      onStored(updated);
      toast.success(t("studio.toast.unpublished"));
    },
    onError,
  });
  const remove = useMutation({
    mutationFn: () => videoClient().deleteVideo({ videoId: summary.id }),
    onSuccess: () => {
      setDeleting(false);
      removeVideoEverywhere(queryClient, summary.id);
      queryClient.removeQueries({ queryKey: videoKeys.video(videoKey) });
      void queryClient.invalidateQueries({ queryKey: videoKeys.channels });
      toast.success(t("studio.toast.deleted"));
      void navigate({ to: "/studio" });
    },
    onError: (error) => {
      setDeleting(false);
      onError(error);
    },
  });

  const lead =
    summary.status === VideoStatus.PUBLISHED
      ? t("studio.status.published")
      : summary.status === VideoStatus.SCHEDULED
        ? summary.processing === ProcessingState.PROCESSING && !summary.scheduledAt
          ? t("studio.status.publishWhenReady")
          : t("studio.status.scheduled")
        : failed
          ? t("studio.status.failed")
          : t("studio.status.draft");

  return (
    <section
      className="sticker rounded-card bg-surface p-4"
      data-testid="video-status"
      aria-label={t("studio.status.title")}
    >
      <VideoStateChips summary={summary} />
      <p className="mt-2 text-sm leading-relaxed text-ink-soft">{lead}</p>
      <div className="mt-3 flex flex-wrap gap-2">
        {summary.status === VideoStatus.DRAFT ? (
          <Button
            size="sm"
            icon={<Rocket className="size-4" />}
            disabled={failed}
            onClick={openPublish}
          >
            {t("studio.publish.action")}
          </Button>
        ) : (
          <>
            {summary.status === VideoStatus.SCHEDULED && (
              <Button size="sm" icon={<Rocket className="size-4" />} onClick={openPublish}>
                {t("studio.publish.change")}
              </Button>
            )}
            <Button
              size="sm"
              variant="secondary"
              icon={<EyeOff className="size-4" />}
              loading={unpublish.isPending}
              onClick={() => unpublish.mutate()}
            >
              {summary.status === VideoStatus.SCHEDULED
                ? t("studio.publish.cancelSchedule")
                : t("studio.publish.unpublish")}
            </Button>
          </>
        )}
        <Link
          to="/watch/$videoId"
          params={{ videoId: summary.shortId }}
          search={{}}
          className="pressable sticker inline-flex h-9 items-center gap-2 rounded-full bg-surface px-4 text-sm font-bold"
        >
          <Play className="size-4" />
          {t("studio.videos.watch")}
        </Link>
        <Button
          variant="ghost"
          size="icon-sm"
          className="ml-auto text-danger"
          aria-label={t("studio.deleteVideo")}
          onClick={() => setDeleting(true)}
        >
          <Trash2 className="size-5" />
        </Button>
      </div>

      <PublishDialog
        key={opened}
        open={publishing}
        onOpenChange={setPublishing}
        summary={summary}
        visibility={draft.visibility}
        circleId={draft.circleId}
        blocked={dirty && problem !== null}
        // What was typed goes with the video: unsaved details are saved before it is published.
        beforePublish={dirty ? saveDraft : undefined}
        onPublished={onStored}
      />
      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={t("studio.confirmDelete", { title: summary.title })}
        description={t("studio.confirmDeleteBody")}
        confirmLabel={t("actions.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </section>
  );
}

/** Publishing: for whom, and when — now, or at a time the creator picks. */
function PublishDialog({
  open,
  onOpenChange,
  summary,
  visibility,
  circleId,
  blocked,
  beforePublish,
  onPublished,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  summary: video.VideoSummary;
  /** The audience chosen in the details form. */
  visibility: common.Visibility;
  /** Its circle, for CIRCLE. */
  circleId: string;
  /** The details have a problem that must be fixed first. */
  blocked: boolean;
  beforePublish?(): Promise<unknown>;
  onPublished(updated: video.Video | undefined): void;
}) {
  const { t } = useTranslation("video");
  // "Publish" on a video that is still private means: show it to people.
  const [audience, setAudience] = useState<common.Visibility | null>(null);
  const chosen =
    audience ?? (visibility === common.Visibility.PRIVATE ? common.Visibility.PUBLIC : visibility);
  const [circle, setCircle] = useState(circleId);
  const [when, setWhen] = useState<"now" | "later">(summary.scheduledAt ? "later" : "now");
  const [at, setAt] = useState(() =>
    toLocalInput(
      summary.scheduledAt
        ? new Date(toMillis(summary.scheduledAt))
        : new Date(Date.now() + 3_600_000),
    ),
  );
  const atDate = new Date(at);
  const validTime =
    when === "now" || (!Number.isNaN(atDate.getTime()) && atDate.getTime() > Date.now());

  const publish = useMutation({
    mutationFn: async () => {
      await beforePublish?.();
      return (
        await videoClient().publishVideo({
          videoId: summary.id,
          visibility: chosen,
          circleId: chosen === common.Visibility.CIRCLE ? circle : "",
          publishAt: when === "later" ? fromDate(atDate) : undefined,
        })
      ).video;
    },
    onSuccess: (updated) => {
      onPublished(updated);
      onOpenChange(false);
      const status = updated?.summary?.status;
      toast.success(
        status === VideoStatus.PUBLISHED
          ? t("studio.toast.published")
          : when === "later"
            ? t("studio.toast.scheduled")
            : t("studio.toast.publishWhenReady"),
      );
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("studio.publish.title")}
      description={
        summary.processing === ProcessingState.PROCESSING
          ? t("studio.publish.leadProcessing")
          : t("studio.publish.lead")
      }
      footer={
        <>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            {t("actions.cancel")}
          </Button>
          <Button
            icon={
              when === "later" ? <CalendarClock className="size-4" /> : <Eye className="size-4" />
            }
            disabled={!validTime || blocked}
            loading={publish.isPending}
            onClick={() => publish.mutate()}
          >
            {when === "later" ? t("studio.publish.schedule") : t("studio.publish.now")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        <SelectField
          label={t("studio.editor.audience")}
          value={String(chosen)}
          onChange={(value) => setAudience(Number(value) as common.Visibility)}
          options={VIDEO_VISIBILITIES.map((option) => ({
            value: String(option.value),
            label: t(`visibility.${option.key}`),
            hint: t(`visibility.${option.key}Hint`),
          }))}
        />
        {chosen === common.Visibility.CIRCLE && (
          <CircleField
            label={t("visibility.circlePick")}
            emptyText={t("visibility.circleNone")}
            value={circle}
            onChange={setCircle}
          />
        )}
        <LockedNote text={t("visibility.lockedNote")} />
        <Tabs<"now" | "later">
          value={when}
          onChange={setWhen}
          items={[
            { value: "now", label: t("studio.publish.whenNow") },
            { value: "later", label: t("studio.publish.whenLater") },
          ]}
        />
        {when === "later" && (
          <TextField
            label={t("studio.publish.at")}
            type="datetime-local"
            value={at}
            min={toLocalInput(new Date())}
            error={validTime ? undefined : t("studio.publish.atInvalid")}
            hint={t("studio.publish.atHint")}
            onChange={(event) => setAt(event.target.value)}
          />
        )}
        {blocked && (
          <p role="alert" className="text-sm font-medium text-danger">
            {t("studio.publish.fixFirst")}
          </p>
        )}
      </div>
    </Dialog>
  );
}
