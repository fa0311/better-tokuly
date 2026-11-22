import { useMutation, useQueryClient } from "@tanstack/react-query";
import { media, type video } from "@tokuly/api";
import { Camera, ImagePlus, RotateCcw } from "lucide-react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { Spinner } from "#/components/ui/spinner";
import { errorMessage } from "#/lib/errors";
import { uploadAndProcess } from "#/lib/upload";
import { patchVideoEverywhere, videoClient, videoKeys } from "../api";
import { type PlayerHandle, VideoPlayer } from "../player/player";
import { imageUrl } from "../video-card";

/** The frame the player is showing, as a JPEG. */
function captureFrame(element: HTMLVideoElement): Promise<Blob> {
  const canvas = document.createElement("canvas");
  canvas.width = element.videoWidth;
  canvas.height = element.videoHeight;
  const context = canvas.getContext("2d");
  if (!context || canvas.width === 0) return Promise.reject(new Error("no frame to capture"));
  context.drawImage(element, 0, 0, canvas.width, canvas.height);
  return new Promise((resolve, reject) =>
    canvas.toBlob(
      (blob) => (blob ? resolve(blob) : reject(new Error("could not encode the frame"))),
      "image/jpeg",
      0.9,
    ),
  );
}

/**
 * The picture a video is known by. It starts as a frame chosen by the
 * server; the creator can upload their own, or stop the video on a moment
 * they like and use that. Each choice is saved at once.
 */
export function ThumbnailField({
  video: item,
  videoKey,
}: {
  video: video.Video /** The ID the editor's query is keyed by. */;
  videoKey: string;
}) {
  const { t } = useTranslation("video");
  const queryClient = useQueryClient();
  const input = useRef<HTMLInputElement>(null);
  const picker = useRef<PlayerHandle>(null);
  const [picking, setPicking] = useState(false);
  const summary = item.summary;
  const current = imageUrl(summary?.thumbnail, 320);

  /** Uploads `image` as the thumbnail, or — with `null` — goes back to the automatic one. */
  const change = useMutation({
    mutationFn: async (image: (Blob & { name?: string }) | null) => {
      const assetId = image
        ? (await uploadAndProcess(image, { purpose: media.AssetPurpose.THUMBNAIL })).id
        : "";
      return (
        await videoClient().updateVideo({ videoId: summary?.id ?? "", thumbnailAssetId: assetId })
      ).video;
    },
    onSuccess: (updated, image) => {
      if (updated) {
        queryClient.setQueryData(videoKeys.video(videoKey), updated);
        if (updated.summary)
          patchVideoEverywhere(queryClient, updated.summary.id, () => updated.summary ?? null);
      }
      setPicking(false);
      toast.success(image ? t("studio.thumbnail.saved") : t("studio.thumbnail.reset"));
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  const captureCurrent = async () => {
    const element = picker.current?.element();
    if (!element) return;
    try {
      const frame = await captureFrame(element);
      change.mutate(Object.assign(frame, { name: "thumbnail.jpg" }));
    } catch {
      toast.error(t("studio.thumbnail.captureFailed"));
    }
  };

  return (
    <div className="flex flex-col gap-3">
      <div
        className="relative aspect-video w-full overflow-hidden rounded-2xl border-2 border-line bg-surface-3"
        data-testid="thumbnail-preview"
      >
        {current ? (
          <img
            src={current}
            alt={t("studio.thumbnail.current")}
            className="size-full object-cover"
          />
        ) : (
          <p className="grid size-full place-items-center px-4 text-center text-sm font-bold text-ink-soft">
            {t("studio.thumbnail.pending")}
          </p>
        )}
        {change.isPending && (
          <span className="absolute inset-0 grid place-items-center bg-overlay text-white">
            <Spinner className="size-7" />
          </span>
        )}
      </div>
      <div className="flex flex-wrap gap-2">
        <Button
          variant="secondary"
          size="sm"
          icon={<ImagePlus className="size-4" />}
          disabled={change.isPending}
          onClick={() => input.current?.click()}
        >
          {t("studio.thumbnail.upload")}
        </Button>
        {item.playback && (
          <Button
            variant="secondary"
            size="sm"
            icon={<Camera className="size-4" />}
            disabled={change.isPending}
            onClick={() => setPicking(true)}
          >
            {t("studio.thumbnail.pick")}
          </Button>
        )}
        {item.studio?.hasCustomThumbnail && (
          <Button
            variant="ghost"
            size="sm"
            icon={<RotateCcw className="size-4" />}
            disabled={change.isPending}
            onClick={() => change.mutate(null)}
          >
            {t("studio.thumbnail.auto")}
          </Button>
        )}
      </div>
      <input
        ref={input}
        type="file"
        accept="image/png,image/jpeg,image/webp"
        className="hidden"
        aria-label={t("studio.thumbnail.upload")}
        onChange={(event) => {
          const file = event.target.files?.[0];
          event.target.value = "";
          if (file) change.mutate(file);
        }}
      />

      {item.playback && (
        <Dialog
          open={picking}
          onOpenChange={setPicking}
          title={t("studio.thumbnail.pickTitle")}
          description={t("studio.thumbnail.pickLead")}
          className="sm:w-[min(44rem,calc(100vw-2rem))]"
          footer={
            <>
              <Button variant="ghost" onClick={() => setPicking(false)}>
                {t("actions.cancel")}
              </Button>
              <Button
                icon={<Camera className="size-4" />}
                loading={change.isPending}
                onClick={() => void captureCurrent()}
              >
                {t("studio.thumbnail.useFrame")}
              </Button>
            </>
          }
        >
          <VideoPlayer ref={picker} source={item.playback} className="rounded-2xl" />
        </Dialog>
      )}
    </div>
  );
}
