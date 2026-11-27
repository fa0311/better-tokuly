import type { media } from "@tokuly/api";
import { AssetFailedError } from "@tokuly/core/errors";
import { Film, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Spinner } from "#/components/ui/spinner";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import i18n from "#/lib/i18n";
import { uploadFile, waitForAsset } from "#/lib/upload";

/** A picture or video picked for a note or a story, on its way to the server or already there. */
export interface Upload {
  key: string;
  kind: "image" | "video";
  /** A local preview of the picked file. */
  previewUrl: string;
  /** 0..1 while the bytes go up. */
  progress: number;
  status: "uploading" | "processing" | "ready";
  /** Set once the asset can be attached. */
  assetId?: string;
}

export interface UploadRules {
  /** The asset purpose a picked file is uploaded with. */
  purpose(kind: Upload["kind"]): media.AssetPurpose;
  maxItems: number;
  /** A video comes alone and pictures do not mix with it (notes). */
  videoAlone?: boolean;
}

const fileKind = (file: File): Upload["kind"] | undefined =>
  file.type.startsWith("image/") ? "image" : file.type.startsWith("video/") ? "video" : undefined;
const say = (key: string, options?: Record<string, unknown>) =>
  i18n.t(key, { ns: "notes", ...options });

/**
 * Pictures and videos being attached: each picked file is uploaded and
 * processed in the background while its preview shows, can be taken out
 * again, and yields an asset ID once ready. Files that break the rules (too
 * many, a video next to pictures, something that is neither) are refused
 * with a toast.
 */
export function useUploads(rules: UploadRules) {
  const [uploads, setUploads] = useState<Upload[]>([]);
  const controllers = useRef(new Map<string, AbortController>());
  // The latest list, for the checks in `add` (several files may arrive before a render). Every change goes through `update`.
  const current = useRef<Upload[]>([]);

  const update = (next: Upload[]) => {
    current.current = next;
    setUploads(next);
  };
  const patch = (key: string, change: Partial<Upload>) =>
    update(
      current.current.map((upload) => (upload.key === key ? { ...upload, ...change } : upload)),
    );

  const remove = (key: string) => {
    controllers.current.get(key)?.abort();
    controllers.current.delete(key);
    const upload = current.current.find((item) => item.key === key);
    if (upload) URL.revokeObjectURL(upload.previewUrl);
    update(current.current.filter((item) => item.key !== key));
  };

  const start = async (upload: Upload, file: File) => {
    const controller = new AbortController();
    controllers.current.set(upload.key, controller);
    try {
      const asset = await uploadFile(file, {
        purpose: rules.purpose(upload.kind),
        signal: controller.signal,
        onProgress: (progress) => patch(upload.key, { progress }),
      });
      patch(upload.key, { status: "processing", progress: 1 });
      const ready = await waitForAsset(asset.id, undefined, controller.signal);
      patch(upload.key, { status: "ready", assetId: ready.id });
    } catch (error) {
      if (controller.signal.aborted) return;
      remove(upload.key);
      toast.error(
        error instanceof AssetFailedError && error.reason === "PROCESSING_FAILED"
          ? say("uploads.failed")
          : errorMessage(error, { namespace: "notes" }),
      );
    } finally {
      controllers.current.delete(upload.key);
    }
  };

  const add = (files: Iterable<File>) => {
    for (const file of files) {
      const kind = fileKind(file);
      const existing = current.current;
      if (!kind) {
        toast.error(say("uploads.unsupported"));
        continue;
      }
      if (existing.length >= rules.maxItems) {
        toast.error(say("uploads.tooMany", { count: rules.maxItems }));
        break;
      }
      if (
        rules.videoAlone &&
        existing.length > 0 &&
        (kind === "video" || existing.some((item) => item.kind === "video"))
      ) {
        toast.error(say("uploads.videoAlone"));
        continue;
      }
      const upload: Upload = {
        key: crypto.randomUUID(),
        kind,
        previewUrl: URL.createObjectURL(file),
        progress: 0,
        status: "uploading",
      };
      update([...existing, upload]);
      void start(upload, file);
    }
  };

  // Leaving the composer cancels what is still on its way and frees the previews.
  useEffect(
    () => () => {
      for (const controller of controllers.current.values()) controller.abort();
      for (const upload of current.current) URL.revokeObjectURL(upload.previewUrl);
    },
    [],
  );

  return {
    uploads,
    add,
    remove,
    /** Puts the uploads in a new order (stories play in it). */
    reorder: (next: Upload[]) => update(next),
    /** Something is still uploading or being processed. */
    busy: uploads.some((upload) => upload.status !== "ready"),
    /** Asset IDs of everything attached, in order; only complete once `busy` is false. */
    assetIds: uploads.flatMap((upload) => (upload.assetId ? [upload.assetId] : [])),
    /** Room for more files, and for which kinds. */
    canAdd:
      uploads.length < rules.maxItems &&
      !(rules.videoAlone && uploads.some((upload) => upload.kind === "video")),
  };
}

/** One attached file: its preview, the upload's progress on top, and a button to take it out. */
export function UploadTile({
  upload,
  onRemove,
  className,
}: {
  upload: Upload;
  onRemove(): void;
  className?: string;
}) {
  const { t } = useTranslation("notes");
  const pending = upload.status !== "ready";
  return (
    <div
      className={cn(
        "relative overflow-hidden rounded-2xl border-2 border-ink bg-surface-3",
        className,
      )}
      data-testid="upload"
      data-status={upload.status}
    >
      {upload.kind === "video" ? (
        <>
          <video
            src={upload.previewUrl}
            muted
            playsInline
            preload="metadata"
            className="size-full object-cover"
          />
          <span className="absolute bottom-1.5 left-1.5 grid size-6 place-items-center rounded-full bg-black/60 text-white">
            <Film className="size-3.5" />
          </span>
        </>
      ) : (
        <img src={upload.previewUrl} alt="" className="size-full object-cover" />
      )}
      {pending && (
        <div
          className="absolute inset-0 grid place-items-center bg-black/45 text-white"
          role="status"
          aria-label={
            upload.status === "uploading" ? t("uploads.uploading") : t("uploads.processing")
          }
        >
          <div className="flex flex-col items-center gap-1.5">
            <Spinner className="size-6" />
            <span className="text-xs font-black tabular-nums">
              {upload.status === "uploading"
                ? `${Math.round(upload.progress * 100)}%`
                : t("uploads.processing")}
            </span>
          </div>
        </div>
      )}
      <button
        type="button"
        aria-label={t("uploads.remove")}
        onClick={onRemove}
        className="absolute top-1.5 right-1.5 grid size-7 place-items-center rounded-full bg-black/65 text-white hover:bg-black/85"
      >
        <X className="size-4" />
      </button>
    </div>
  );
}
