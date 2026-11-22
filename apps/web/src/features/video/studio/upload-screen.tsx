import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useBlocker, useNavigate } from "@tanstack/react-router";
import { media } from "@tokuly/api";
import { CircleCheck, FileVideo, Plus, UploadCloud, X } from "lucide-react";
import { type DragEvent, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { TextArea, TextField } from "#/components/ui/field";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { uploadFile } from "#/lib/upload";
import { videoClient, videoKeys } from "../api";
import { ChannelPicker, FormSection, selectStudioChannel, useStudioChannel } from "./shared";

interface Upload {
  file: File;
  /** 0…1 of the file's bytes that reached storage. */
  progress: number;
  /** Set once the file is up: the upload the video will play. */
  assetId?: string;
  failed?: string;
}

/** A file name without its extension, as a first title. */
function titleFromFile(name: string): string {
  return name
    .replace(/\.[A-Za-z0-9]{2,5}$/, "")
    .replace(/[_-]+/g, " ")
    .trim()
    .slice(0, 100);
}

function fileSize(bytes: number): string {
  return bytes >= 1024 ** 3 ? `${(bytes / 1024 ** 3).toFixed(1)} GB` : `${Math.max(0.1, bytes / 1024 ** 2).toFixed(1)} MB`;
}

/**
 * `/studio/upload`: choose a video and it starts going up at once; the title
 * and description can be typed meanwhile. When the file is up a draft is
 * made and the editor opens, where processing is followed live and the
 * video is published.
 */
export function UploadScreen() {
  const { t } = useTranslation("video");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { query, channels, channel } = useStudioChannel();
  const input = useRef<HTMLInputElement>(null);
  const abort = useRef<AbortController | null>(null);
  const [upload, setUpload] = useState<Upload | null>(null);
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [dragging, setDragging] = useState(false);

  // Leaving the page gives the upload up — so while one is under way (or up but not yet a draft),
  // leaving asks first: inside the app with a dialog, for a reload or a closed tab with the browser's own.
  useEffect(() => () => abort.current?.abort(), []);
  // Set the moment the upload became a draft: going on to the editor is not "leaving".
  const handedOver = useRef(false);
  const atRisk = () => upload !== null && !upload.failed && !handedOver.current;
  const blocker = useBlocker({ shouldBlockFn: atRisk, enableBeforeUnload: atRisk, withResolver: true });

  const start = async (file: File) => {
    if (!file.type.startsWith("video/") && !/\.(mp4|mov|m4v|webm|mkv|avi)$/i.test(file.name)) {
      toast.error(t("studio.upload.notVideo"));
      return;
    }
    abort.current?.abort();
    const controller = new AbortController();
    abort.current = controller;
    setUpload({ file, progress: 0 });
    setTitle((current) => current || titleFromFile(file.name));
    try {
      const asset = await uploadFile(file, {
        purpose: media.AssetPurpose.VIDEO,
        signal: controller.signal,
        onProgress: (progress) => setUpload((current) => (current?.file === file ? { ...current, progress } : current)),
      });
      setUpload((current) => (current?.file === file ? { ...current, progress: 1, assetId: asset.id } : current));
    } catch (error) {
      if (controller.signal.aborted) return;
      setUpload((current) => (current?.file === file ? { ...current, failed: errorMessage(error) } : current));
    }
  };
  const cancel = () => {
    abort.current?.abort();
    setUpload(null);
  };

  const createDraft = useMutation({
    mutationFn: () => videoClient().createVideo({ channelId: channel?.summary?.id ?? "", assetId: upload?.assetId ?? "", title: title.trim(), description: description.trim() }),
    onSuccess: ({ video: created }) => {
      const summary = created?.summary;
      if (!created || !summary) return;
      // The editor opens with what was just created; the lists pick the new video up.
      handedOver.current = true;
      queryClient.setQueryData(videoKeys.video(summary.shortId), created);
      void queryClient.invalidateQueries({ queryKey: videoKeys.studio });
      void navigate({ to: "/studio/videos/$videoId", params: { videoId: summary.shortId } });
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  const onDrop = (event: DragEvent) => {
    event.preventDefault();
    setDragging(false);
    const file = event.dataTransfer.files[0];
    if (file) void start(file);
  };

  if (query.isPending) {
    return (
      <Page title={t("studio.upload.title")} back="/studio">
        <Skeleton className="h-72 rounded-card" />
      </Page>
    );
  }
  if (!channel) {
    return (
      <Page title={t("studio.upload.title")} back="/studio">
        <EmptyState
          emoji="📡"
          title={t("studio.upload.needChannel")}
          body={t("studio.upload.needChannelBody")}
          action={
            <Link to="/studio/channels/new" className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-brand px-5 font-bold text-on-brand">
              <Plus className="size-4" />
              {t("studio.welcome.action")}
            </Link>
          }
        />
      </Page>
    );
  }

  const percent = Math.round((upload?.progress ?? 0) * 100);
  const uploaded = Boolean(upload?.assetId);

  return (
    <Page title={t("studio.upload.title")} back="/studio">
      <div className="flex flex-col gap-4">
        <ChannelPicker channels={channels} selectedId={channel.summary?.id} onSelect={selectStudioChannel} />
        {channels.length > 1 && <p className="px-1 text-sm text-ink-soft">{t("studio.upload.toChannel", { name: channel.summary?.name ?? "" })}</p>}

        <input
          ref={input}
          type="file"
          accept="video/*,.mkv"
          className="hidden"
          data-testid="video-file"
          aria-label={t("studio.upload.choose")}
          onChange={(event) => {
            const file = event.target.files?.[0];
            event.target.value = "";
            if (file) void start(file);
          }}
        />

        {!upload ? (
          <button
            type="button"
            className={cn(
              "flex flex-col items-center gap-3 rounded-card border-[3px] border-dashed px-6 py-14 text-center transition-colors",
              dragging ? "border-brand bg-brand-soft" : "border-line bg-surface hover:border-ink-faint",
            )}
            onClick={() => input.current?.click()}
            onDragOver={(event) => {
              event.preventDefault();
              setDragging(true);
            }}
            onDragLeave={() => setDragging(false)}
            onDrop={onDrop}
          >
            <span className="sticker grid size-16 place-items-center rounded-3xl bg-brand text-on-brand">
              <UploadCloud className="size-8" />
            </span>
            <span className="text-xl font-black">{t("studio.upload.choose")}</span>
            <span className="max-w-xs text-sm leading-relaxed text-ink-soft">{t("studio.upload.hint")}</span>
          </button>
        ) : (
          <>
            <section className="sticker rounded-card bg-surface p-4" aria-label={t("studio.upload.progress")}>
              <div className="flex items-center gap-3">
                <span className={cn("grid size-11 shrink-0 place-items-center rounded-2xl border-2 border-ink", uploaded ? "bg-lime text-[#17122b]" : "bg-surface-2")}>{uploaded ? <CircleCheck className="size-6" /> : <FileVideo className="size-6" />}</span>
                <div className="min-w-0 flex-1">
                  <p className="truncate font-bold">{upload.file.name}</p>
                  <p className="text-sm text-ink-soft" role="status" data-testid="upload-status">
                    {upload.failed ? upload.failed : uploaded ? t("studio.upload.done", { size: fileSize(upload.file.size) }) : t("studio.upload.uploading", { percent, size: fileSize(upload.file.size) })}
                  </p>
                </div>
                <Button variant="ghost" size="icon-sm" aria-label={t("studio.upload.cancel")} onClick={cancel}>
                  <X className="size-5" />
                </Button>
              </div>
              <div
                className="mt-3 h-3 overflow-hidden rounded-full border-2 border-ink bg-surface-3"
                role="progressbar"
                aria-label={t("studio.upload.progress")}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={percent}
              >
                <div className={cn("h-full transition-[width] duration-200", upload.failed ? "bg-danger" : "bg-lime")} style={{ width: `${percent}%` }} />
              </div>
              {upload.failed && (
                <Button variant="secondary" className="mt-3" onClick={() => void start(upload.file)}>
                  {t("actions.retry")}
                </Button>
              )}
            </section>

            <FormSection title={t("studio.upload.detailsTitle")} lead={t("studio.upload.detailsLead")}>
              <TextField label={t("studio.editor.title")} value={title} maxLength={100} placeholder={t("studio.editor.titlePlaceholder")} onChange={(event) => setTitle(event.target.value)} />
              <TextArea label={t("studio.editor.description")} value={description} maxLength={5000} placeholder={t("studio.editor.descriptionPlaceholder")} onChange={(event) => setDescription(event.target.value)} />
            </FormSection>

            <div className="flex justify-end">
              <Button size="lg" disabled={!uploaded} loading={createDraft.isPending} onClick={() => createDraft.mutate()}>
                {uploaded ? t("studio.upload.next") : t("studio.upload.waiting", { percent })}
              </Button>
            </div>
          </>
        )}
      </div>
      <ConfirmDialog
        open={blocker.status === "blocked"}
        onOpenChange={(open) => !open && blocker.reset?.()}
        title={t("studio.upload.leaveTitle")}
        description={t("studio.upload.leaveBody")}
        confirmLabel={t("studio.upload.leave")}
        danger
        onConfirm={() => blocker.proceed?.()}
      />
    </Page>
  );
}
