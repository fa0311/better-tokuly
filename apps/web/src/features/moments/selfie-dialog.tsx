import type { moments } from "@tokuly/api";
import { realmojiGlyph } from "@tokuly/core/moments";
import { Circle, ImagePlus } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { useTranslation } from "#/lib/use-translation";
import { openCamera, snap, stopStream } from "./camera";
import { faceKey } from "./faces";
import { LiveVideo } from "./live-video";

/**
 * Takes one selfie for a RealMoji: the front camera, or a picture picked
 * where there is none. `onShot` gets the picture; the dialog closes itself.
 */
export function SelfieDialog({
  emoji,
  onShot,
  onClose,
}: {
  emoji: moments.RealmojiEmoji | null;
  onShot(picture: File): void;
  onClose(): void;
}) {
  const { t } = useTranslation("moments");
  const face = emoji === null ? "" : `${realmojiGlyph(emoji)} ${t(faceKey(emoji))}`;
  return (
    <Dialog
      open={emoji !== null}
      onOpenChange={(open) => !open && onClose()}
      title={t("selfie.title")}
      description={t("selfie.lead", { face })}
    >
      {emoji !== null && (
        <Selfie
          onShot={(picture) => {
            onShot(picture);
            onClose();
          }}
        />
      )}
    </Dialog>
  );
}

function Selfie({ onShot }: { onShot(picture: File): void }) {
  const { t } = useTranslation("moments");
  const video = useRef<HTMLVideoElement | null>(null);
  const picker = useRef<HTMLInputElement>(null);
  const [stream, setStream] = useState<MediaStream | null>(null);
  const [unavailable, setUnavailable] = useState(false);
  const [live, setLive] = useState(false);

  useEffect(() => {
    let cancelled = false;
    let opened: MediaStream | undefined;
    if (!navigator.mediaDevices?.getUserMedia) {
      setUnavailable(true);
      return;
    }
    openCamera("user").then(
      (next) => {
        opened = next;
        if (cancelled) return stopStream(next);
        setStream(next);
      },
      () => !cancelled && setUnavailable(true),
    );
    return () => {
      cancelled = true;
      stopStream(opened);
    };
  }, []);

  const shoot = async () => {
    if (!video.current) return;
    const blob = await snap(video.current, true);
    if (blob) onShot(new File([blob], `realmoji-${Date.now()}.jpg`, { type: "image/jpeg" }));
  };

  return (
    <div className="flex flex-col items-center gap-4">
      {unavailable || !stream ? (
        <div className="flex aspect-square w-full max-w-72 flex-col items-center justify-center gap-3 rounded-full bg-surface-2 p-6 text-center">
          {unavailable && <p className="text-sm text-ink-soft">{t("selfie.unavailable")}</p>}
          <Button
            variant="secondary"
            icon={<ImagePlus className="size-4" />}
            onClick={() => picker.current?.click()}
          >
            {t("selfie.pick")}
          </Button>
        </div>
      ) : (
        <LiveVideo
          stream={stream}
          mirror
          videoRef={video}
          onLive={() => setLive(true)}
          className="aspect-square w-full max-w-72 rounded-full border-[3px] border-ink"
          testId="selfie-preview"
        />
      )}
      {stream && (
        <button
          type="button"
          onClick={() => void shoot()}
          disabled={!live}
          aria-label={t("selfie.shoot")}
          className="pressable sticker grid size-16 place-items-center rounded-full bg-surface text-ink disabled:opacity-40"
        >
          <Circle className="size-9" />
        </button>
      )}
      <input
        ref={picker}
        type="file"
        accept="image/*"
        capture="user"
        className="hidden"
        data-testid="selfie-pick"
        onChange={(event) => {
          const file = event.target.files?.[0];
          if (file?.type.startsWith("image/")) onShot(file);
          event.target.value = "";
        }}
      />
    </div>
  );
}
