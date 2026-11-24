import { create } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { common, live, media, toMillis, video } from "@tokuly/api";
import { ImagePlus, Trash2 } from "lucide-react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { TextArea, TextField } from "#/components/ui/field";
import { Page } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { PaneSpinner, Spinner } from "#/components/ui/spinner";
import { SwitchRow } from "#/components/ui/switch";
import { CircleField, LockedNote } from "#/features/video/audience";
import { useAccountTimeZone, zonedMoment, zonedTime } from "#/lib/account-time";
import { errorMessage } from "#/lib/errors";
import { uploadAndProcess } from "#/lib/upload";
import { storeStream, streamClient, useStream } from "../api";
import { ChatSettingsFields, defaultChatSettings } from "../chat/chat-settings";
import { CATEGORIES, VISIBILITIES } from "../labels";
import { imageUrl } from "../parts";
import { LiveChannelPicker, NoChannel, Section, useLiveChannel } from "./shared";

const { StreamStatus } = live;

interface Draft {
  title: string;
  description: string;
  tags: string;
  category: video.VideoCategory;
  game: string;
  visibility: common.Visibility;
  /** For CIRCLE: one of the caller's circles. */
  circleId: string;
  scheduled: boolean;
  /** The planned start on the account's clock: `YYYY-MM-DD` and `HH:MM`. */
  date: string;
  time: string;
  lowLatency: boolean;
  dvrEnabled: boolean;
  autoArchive: boolean;
  autoStart: boolean;
  giftsEnabled: boolean;
  /** `undefined` = unchanged, `""` = removed, otherwise the upload to use. */
  thumbnailAssetId: string | undefined;
  thumbnailPreview: string | undefined;
  chat: live.ChatSettings;
}

const pad = (value: number) => String(value).padStart(2, "0");

function clockParts(ms: number, zone: string): { date: string; time: string } {
  const at = zonedTime(ms, zone);
  return { date: at.date, time: `${pad(Math.floor(at.minute / 60))}:${pad(at.minute % 60)}` };
}

function draftOf(stream: live.Stream | undefined, zone: string): Draft {
  const summary = stream?.summary;
  const scheduledMs = summary?.scheduledAt ? toMillis(summary.scheduledAt) : undefined;
  // A new schedule starts at the next full hour.
  const suggestion = Math.ceil((Date.now() + 30 * 60_000) / 3_600_000) * 3_600_000;
  return {
    title: summary?.title ?? "",
    description: stream?.description ?? "",
    tags: stream?.tags.join(", ") ?? "",
    category: summary?.category ?? video.VideoCategory.UNSPECIFIED,
    game: summary?.game ?? "",
    visibility: summary?.visibility || common.Visibility.PUBLIC,
    circleId: stream?.studio?.circleId ?? "",
    scheduled: scheduledMs !== undefined,
    ...clockParts(scheduledMs ?? suggestion, zone),
    lowLatency: summary?.lowLatency ?? true,
    dvrEnabled: stream?.dvrEnabled ?? true,
    autoArchive: stream?.autoArchive ?? true,
    autoStart: stream?.studio?.autoStart ?? true,
    giftsEnabled: stream?.giftsEnabled ?? true,
    thumbnailAssetId: undefined,
    thumbnailPreview: imageUrl(summary?.thumbnail, 320),
    chat: stream?.chat ?? defaultChatSettings(),
  };
}

function scheduledMs(draft: Draft, zone: string): number | undefined {
  if (!draft.scheduled || !draft.date || !draft.time) return undefined;
  const [hours = 0, minutes = 0] = draft.time.split(":").map(Number);
  return zonedMoment(draft.date, hours * 60 + minutes, zone);
}

const tagsOf = (text: string) =>
  text
    .split(/[,、\s]+/)
    .map((tag) => tag.replace(/^#/, "").trim())
    .filter(Boolean);

/**
 * Everything a stream is set up with: what it is called and about, who may
 * watch, when it starts, how it is delivered and how its chat behaves. Used
 * for a new stream and for changing one.
 */
function StreamForm({ stream, channelId }: { stream?: live.Stream; channelId: string }) {
  const { t } = useTranslation("live");
  const zone = useAccountTimeZone();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [draft, setDraft] = useState(() => draftOf(stream, zone));
  const set = (patch: Partial<Draft>) => setDraft((current) => ({ ...current, ...patch }));
  const file = useRef<HTMLInputElement>(null);
  const editing = stream !== undefined;
  const started = editing && stream.summary?.status !== StreamStatus.UPCOMING;
  const ended = stream?.summary?.status === StreamStatus.ENDED;

  const upload = useMutation({
    mutationFn: (image: File) => uploadAndProcess(image, { purpose: media.AssetPurpose.THUMBNAIL }),
    onSuccess: (asset, image) =>
      set({ thumbnailAssetId: asset.id, thumbnailPreview: URL.createObjectURL(image) }),
    onError: (error) => toast.error(errorMessage(error, { namespace: "live" })),
  });

  const save = useMutation({
    mutationFn: async () => {
      const at = scheduledMs(draft, zone);
      const basics = {
        title: draft.title.trim(),
        description: draft.description.trim(),
        category: draft.category,
        game: draft.game.trim(),
        visibility: draft.visibility,
        circleId: draft.visibility === common.Visibility.CIRCLE ? draft.circleId : "",
      };
      if (!editing) {
        return (
          await streamClient().createStream({
            ...basics,
            channelId,
            tags: tagsOf(draft.tags),
            thumbnailAssetId: draft.thumbnailAssetId ?? "",
            scheduledAt: at === undefined ? undefined : timestampFromMs(at),
            lowLatency: draft.lowLatency,
            dvrEnabled: draft.dvrEnabled,
            autoArchive: draft.autoArchive,
            autoStart: draft.autoStart,
            giftsEnabled: draft.giftsEnabled,
            chat: draft.chat,
          })
        ).stream;
      }
      // An ended stream keeps accepting its title, description and visibility; the rest is history.
      if (ended)
        return (
          await streamClient().updateStream({
            streamId: stream.summary?.id ?? "",
            title: basics.title,
            description: basics.description,
            visibility: basics.visibility,
            circleId: basics.circleId,
            clearSchedule: false,
          })
        ).stream;
      return (
        await streamClient().updateStream({
          ...basics,
          streamId: stream.summary?.id ?? "",
          tags: create(live.TagListSchema, { tags: tagsOf(draft.tags) }),
          thumbnailAssetId: draft.thumbnailAssetId,
          // The schedule and auto-start are fixed once the stream is on air.
          scheduledAt: started || at === undefined ? undefined : timestampFromMs(at),
          clearSchedule: !started && !draft.scheduled && Boolean(stream.summary?.scheduledAt),
          lowLatency: draft.lowLatency,
          dvrEnabled: draft.dvrEnabled,
          autoArchive: draft.autoArchive,
          autoStart: started ? undefined : draft.autoStart,
          giftsEnabled: draft.giftsEnabled,
        })
      ).stream;
    },
    onSuccess: (saved) => {
      storeStream(queryClient, saved);
      toast.success(t(editing ? "form.saved" : "form.created"));
      if (saved?.summary)
        void navigate({
          to: "/studio/live/$streamId",
          params: { streamId: saved.summary.shortId },
        });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "live" })),
  });

  return (
    <form
      className="flex flex-col gap-4"
      onSubmit={(event) => {
        event.preventDefault();
        if (draft.title.trim()) save.mutate();
      }}
    >
      <Section title={t("form.about")}>
        <TextField
          label={t("form.title")}
          value={draft.title}
          maxLength={100}
          required
          placeholder={t("form.titlePlaceholder")}
          onChange={(event) => set({ title: event.target.value })}
        />
        <TextArea
          label={t("form.description")}
          value={draft.description}
          maxLength={5000}
          onChange={(event) => set({ description: event.target.value })}
        />
        {!ended && (
          <>
            <div className="grid gap-4 sm:grid-cols-2">
              <SelectField
                label={t("form.category")}
                value={String(draft.category)}
                onChange={(value) => set({ category: Number(value) as video.VideoCategory })}
                options={[
                  { value: "0", label: t("form.noCategory") },
                  ...CATEGORIES.map((category) => ({
                    value: String(category.value),
                    label: `${category.emoji} ${t(`category.${category.key}`)}`,
                  })),
                ]}
              />
              <TextField
                label={t("form.game")}
                value={draft.game}
                maxLength={100}
                placeholder={t("form.gamePlaceholder")}
                onChange={(event) => set({ game: event.target.value })}
              />
            </div>
            <TextField
              label={t("form.tags")}
              hint={t("form.tagsHint")}
              value={draft.tags}
              onChange={(event) => set({ tags: event.target.value })}
            />
            <div className="flex flex-col gap-1.5">
              <span className="px-1 text-sm font-bold text-ink-soft">{t("form.thumbnail")}</span>
              <div className="flex flex-wrap items-center gap-3">
                <span className="relative block aspect-video w-40 overflow-hidden rounded-2xl border-2 border-line bg-surface-3">
                  {draft.thumbnailPreview ? (
                    <img src={draft.thumbnailPreview} alt="" className="size-full object-cover" />
                  ) : (
                    <span className="grid size-full place-items-center text-ink-faint">
                      <ImagePlus className="size-6" />
                    </span>
                  )}
                  {upload.isPending && (
                    <span className="absolute inset-0 grid place-items-center bg-overlay">
                      <Spinner className="size-6 text-white" />
                    </span>
                  )}
                </span>
                <input
                  ref={file}
                  type="file"
                  accept="image/*"
                  className="sr-only"
                  aria-label={t("form.thumbnailChoose")}
                  onChange={(event) => {
                    const image = event.target.files?.[0];
                    if (image) upload.mutate(image);
                    event.target.value = "";
                  }}
                />
                <Button
                  variant="secondary"
                  size="sm"
                  loading={upload.isPending}
                  onClick={() => file.current?.click()}
                >
                  {t("form.thumbnailChoose")}
                </Button>
                {draft.thumbnailPreview && (
                  <Button
                    variant="ghost"
                    size="sm"
                    icon={<Trash2 className="size-4" />}
                    onClick={() => set({ thumbnailAssetId: "", thumbnailPreview: undefined })}
                  >
                    {t("form.thumbnailRemove")}
                  </Button>
                )}
              </div>
            </div>
          </>
        )}
      </Section>

      <Section title={t("form.audience")} lead={t("form.audienceLead")}>
        <SelectField
          label={t("form.visibility")}
          value={String(draft.visibility)}
          onChange={(value) => set({ visibility: Number(value) as common.Visibility })}
          options={VISIBILITIES.map((option) => ({
            value: String(option.value),
            label: t(`visibility.${option.key}`),
            hint: t(`visibility.${option.key}Hint`),
          }))}
        />
        {draft.visibility === common.Visibility.CIRCLE && (
          <CircleField
            label={t("visibility.circlePick")}
            emptyText={t("visibility.circleNone")}
            value={draft.circleId}
            onChange={(circleId) => set({ circleId })}
          />
        )}
        <LockedNote text={t("visibility.lockedNote")} />
      </Section>

      {!started && (
        <Section title={t("form.schedule")} lead={t("form.scheduleLead")}>
          <SwitchRow
            label={t("form.scheduled")}
            hint={t("form.scheduledHint")}
            checked={draft.scheduled}
            onChange={(scheduled) => set({ scheduled })}
          />
          {draft.scheduled && (
            <div className="grid gap-4 sm:grid-cols-2">
              <TextField
                label={t("form.date")}
                type="date"
                value={draft.date}
                required
                onChange={(event) => set({ date: event.target.value })}
              />
              <TextField
                label={t("form.time")}
                type="time"
                value={draft.time}
                required
                hint={t("form.timeZoneHint", { zone })}
                onChange={(event) => set({ time: event.target.value })}
              />
            </div>
          )}
          <SwitchRow
            label={t("form.autoStart")}
            hint={t("form.autoStartHint")}
            checked={draft.autoStart}
            onChange={(autoStart) => set({ autoStart })}
          />
        </Section>
      )}

      {!ended && (
        <Section title={t("form.delivery")}>
          <div className="-mx-3 flex flex-col">
            <SwitchRow
              label={t("form.lowLatency")}
              hint={t("form.lowLatencyHint")}
              checked={draft.lowLatency}
              onChange={(lowLatency) => set({ lowLatency })}
            />
            <SwitchRow
              label={t("form.dvr")}
              hint={t("form.dvrHint")}
              checked={draft.dvrEnabled}
              onChange={(dvrEnabled) => set({ dvrEnabled })}
            />
            <SwitchRow
              label={t("form.autoArchive")}
              hint={t("form.autoArchiveHint")}
              checked={draft.autoArchive}
              onChange={(autoArchive) => set({ autoArchive })}
            />
            <SwitchRow
              label={t("form.gifts")}
              hint={t("form.giftsHint")}
              checked={draft.giftsEnabled}
              onChange={(giftsEnabled) => set({ giftsEnabled })}
            />
          </div>
        </Section>
      )}

      {!editing && (
        <Section title={t("chatSettings.title")} lead={t("form.chatLead")}>
          <div className="-mx-3">
            <ChatSettingsFields value={draft.chat} onChange={(chat) => set({ chat })} />
          </div>
        </Section>
      )}

      <Button
        type="submit"
        size="lg"
        loading={save.isPending}
        disabled={!draft.title.trim() || upload.isPending}
      >
        {t(editing ? "form.save" : draft.scheduled ? "form.createScheduled" : "form.create")}
      </Button>
    </form>
  );
}

/** `/studio/live/new`: set up a stream to start now or at a planned time. */
export function NewStreamScreen() {
  const { t } = useTranslation("live");
  const { query, channels, channel } = useLiveChannel();
  return (
    <Page title={t("form.newTitle")} back="/studio/live">
      {query.isPending ? (
        <PaneSpinner />
      ) : !channel?.summary ? (
        <NoChannel />
      ) : (
        <>
          <LiveChannelPicker channels={channels} selectedId={channel.summary.id} />
          <StreamForm key={channel.summary.id} channelId={channel.summary.id} />
        </>
      )}
    </Page>
  );
}

/** `/studio/live/<id>/edit`: change a stream of the caller. */
export function EditStreamScreen({ streamId }: { streamId: string }) {
  const { t } = useTranslation("live");
  const query = useStream(streamId);
  const stream = query.data;
  return (
    <Page title={t("form.editTitle")} back={`/studio/live/${streamId}`}>
      {query.isPending ? (
        <PaneSpinner />
      ) : stream?.viewer?.isOwner ? (
        <StreamForm
          key={stream.summary?.id}
          stream={stream}
          channelId={stream.summary?.channel?.id ?? ""}
        />
      ) : (
        <p className="py-10 text-center font-bold text-ink-soft">{t("studio.notYours")}</p>
      )}
    </Page>
  );
}
