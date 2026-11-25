import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { media, shorts } from "@tokuly/api";
import { imageUrl, soundTitle } from "@tokuly/core/shorts";
import { BarChart3, Music2, Play, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { errorMessage } from "#/lib/errors";
import { uploadAndProcess } from "#/lib/upload";
import {
  patchShortEverywhere,
  removeShortEverywhere,
  shortClient,
  shortKeys,
  useShort,
} from "../api";
import { CaptionField } from "./caption-field";
import { type CoverChoice, CoverPicker } from "./cover-picker";
import { FormSection } from "./form-section";
import { useProcessingWatch } from "./my-shorts-screen";
import { circleOf, SettingsFields, type ShortSettings } from "./settings-fields";

const { ShortStatus, CoverKind } = shorts;

/** `/shorts/studio/<id>`: change a short's caption, cover and audience, or delete it. */
export function EditScreen({ shortId }: { shortId: string }) {
  const { t } = useTranslation("shorts");
  const query = useShort(shortId);
  const short = query.data;
  useProcessingWatch(short);
  return (
    <Page title={t("edit.title")} back="/shorts/studio">
      {query.isPending ? (
        <Skeleton className="h-72 rounded-card" />
      ) : !short?.studio ? (
        // Someone else's short, or one that is gone: there is nothing to edit.
        <EmptyState emoji="🫥" title={t("edit.missing")} />
      ) : (
        <EditForm key={short.id} short={short} />
      )}
    </Page>
  );
}

function EditForm({ short }: { short: shorts.Short }) {
  const { t } = useTranslation("shorts");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [caption, setCaption] = useState(short.caption);
  const [settings, setSettings] = useState<ShortSettings>({
    visibility: short.visibility,
    circleId: short.studio?.circleId ?? "",
    commentsEnabled: short.commentsEnabled,
    allowRemix: short.allowRemix,
    allowDuet: short.allowDuet,
  });
  // `undefined` = keep the cover as it is.
  const [cover, setCover] = useState<CoverChoice | undefined>(undefined);
  const [deleting, setDeleting] = useState(false);
  const published = short.status === ShortStatus.PUBLISHED;
  const videoUrl = short.playback?.mp4Url ?? "";
  const current: CoverChoice =
    short.coverKind === CoverKind.FRAME
      ? { kind: "frame", seconds: (short.studio?.coverOffsetMs ?? 0) / 1000 }
      : { kind: "poster" };

  const dirty =
    caption !== short.caption ||
    cover !== undefined ||
    settings.visibility !== short.visibility ||
    circleOf(settings) !== (short.studio?.circleId ?? "") ||
    settings.commentsEnabled !== short.commentsEnabled ||
    settings.allowRemix !== short.allowRemix ||
    settings.allowDuet !== short.allowDuet;

  const save = useMutation({
    mutationFn: async () => {
      const coverInput =
        cover?.kind === "frame"
          ? { cover: { case: "offsetMs" as const, value: Math.round(cover.seconds * 1000) } }
          : cover?.kind === "poster"
            ? { cover: { case: "poster" as const, value: true } }
            : cover?.kind === "image"
              ? {
                  cover: {
                    case: "assetId" as const,
                    value: (
                      await uploadAndProcess(cover.file, { purpose: media.AssetPurpose.THUMBNAIL })
                    ).id,
                  },
                }
              : undefined;
      return shortClient().updateShort({
        shortId: short.id,
        caption: caption.trim(),
        visibility: settings.visibility,
        circleId: circleOf(settings),
        commentsEnabled: settings.commentsEnabled,
        allowRemix: settings.allowRemix,
        allowDuet: settings.allowDuet,
        cover: coverInput,
      });
    },
    onSuccess: ({ short: updated }) => {
      if (updated) {
        patchShortEverywhere(queryClient, short.id, () => updated);
        setCaption(updated.caption);
      }
      setCover(undefined);
      void queryClient.invalidateQueries({ queryKey: shortKeys.lists });
      toast.success(t("edit.saved"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "shorts" })),
  });
  const remove = useMutation({
    mutationFn: () => shortClient().deleteShort({ shortId: short.id }),
    onSuccess: () => {
      removeShortEverywhere(queryClient, short.id);
      toast.success(t("studio.deleted"));
      void navigate({ to: "/shorts/studio" });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "shorts" })),
  });

  const coverUrl = imageUrl(short.cover, 120);
  return (
    <div className="flex flex-col gap-4">
      <section className="sticker flex items-center gap-4 rounded-card bg-surface p-4">
        <span
          className="relative block aspect-[9/16] w-20 shrink-0 overflow-hidden rounded-2xl border-2 border-ink bg-surface-3"
          style={short.cover?.dominantColor ? { background: short.cover.dominantColor } : undefined}
        >
          {coverUrl && <img src={coverUrl} alt="" className="size-full object-cover" />}
        </span>
        <div className="min-w-0 flex-1">
          <p className="font-black" data-testid="edit-status">
            {published
              ? t("edit.live")
              : short.status === ShortStatus.FAILED
                ? t("edit.failed")
                : t("edit.processing")}
          </p>
          <p className="mt-0.5 text-sm text-ink-soft">
            {short.status === ShortStatus.FAILED
              ? t(`errors.${short.studio?.processingError || "PROCESSING_FAILED"}`, {
                  defaultValue: t("errors.PROCESSING_FAILED"),
                })
              : published
                ? short.studio?.coverPending
                  ? t("edit.coverPending")
                  : t("edit.liveBody")
                : t("edit.processingBody")}
          </p>
          {published && (
            <div className="mt-2 flex flex-wrap gap-2">
              <Link
                to="/shorts/$shortId"
                params={{ shortId: short.shortId }}
                className="pressable sticker-sm inline-flex h-9 items-center gap-1.5 rounded-full bg-surface px-4 text-sm font-bold"
              >
                <Play className="size-4" />
                {t("studio.watch")}
              </Link>
              <Link
                to="/shorts/studio/analytics"
                search={{ short: short.shortId }}
                className="pressable sticker-sm inline-flex h-9 items-center gap-1.5 rounded-full bg-surface px-4 text-sm font-bold"
              >
                <BarChart3 className="size-4" />
                {t("studio.numbers")}
              </Link>
            </div>
          )}
        </div>
      </section>

      <FormSection title={t("create.details")}>
        <CaptionField value={caption} onChange={setCaption} />
        {short.sound && (
          <Link
            to="/sound/$soundId"
            params={{ soundId: short.sound.id }}
            className="flex items-center gap-2 text-sm font-bold text-ink-soft hover:text-ink"
          >
            <Music2 className="size-4 shrink-0" aria-hidden />
            <span className="truncate">
              <EmojiText text={soundTitle(short.sound, (name) => t("sound.original", { name }))} />
            </span>
          </Link>
        )}
      </FormSection>

      {published && videoUrl && (
        <FormSection title={t("form.cover.title")}>
          <CoverPicker
            src={videoUrl}
            from={0}
            to={short.durationSeconds}
            crossOrigin
            value={cover ?? current}
            onChange={setCover}
          />
        </FormSection>
      )}

      <FormSection title={t("form.audience")}>
        <SettingsFields value={settings} onChange={setSettings} />
      </FormSection>

      <div className="flex flex-wrap justify-between gap-2">
        <Button
          variant="ghost"
          className="text-danger"
          icon={<Trash2 className="size-5" />}
          onClick={() => setDeleting(true)}
        >
          {t("studio.delete")}
        </Button>
        <Button size="lg" disabled={!dirty} loading={save.isPending} onClick={() => save.mutate()}>
          {t("edit.save")}
        </Button>
      </div>
      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={t("studio.deleteTitle")}
        description={t("studio.deleteBody")}
        confirmLabel={t("studio.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </div>
  );
}
