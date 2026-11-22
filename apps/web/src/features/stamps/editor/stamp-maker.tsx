import { useQueryClient } from "@tanstack/react-query";
import { media } from "@tokuly/api";
import { ImagePlus } from "lucide-react";
import { type DragEvent, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { cn } from "#/lib/cn";
import { uploadAndProcess } from "#/lib/upload";
import { libraryClient, refreshHoldings, uploadErrorMessage } from "../api";
import {
  EMPTY_META,
  nameFromFile,
  parseKeywords,
  type StampMeta,
  StampMetaFields,
} from "../stamp-meta";
import { CutoutEditor } from "./cutout-editor";

/** A finished cut-out, uploaded and ready to become a stamp. */
export interface MadeStamp {
  assetId: string;
  name: string;
  emoji: string;
  keywords: string[];
}

export interface StampMakerProps {
  open: boolean;
  onOpenChange(open: boolean): void;
  /** Heading of the last step, e.g. "マイスタンプに追加". */
  title: string;
  /** Label of the button that finishes. */
  submitLabel: string;
  /** Turns the uploaded cut-out into a stamp. The dialog closes when this resolves and shows the error when it rejects. */
  onSubmit(stamp: MadeStamp): Promise<unknown>;
}

type Step = "pick" | "edit" | "details";

/**
 * Makes a stamp from a picture in three steps: choose (or drop) an image,
 * cut it out in the editor, give it a name. What the stamp is added to — the
 * personal collection or a creator pack — is up to `onSubmit`.
 */
export function StampMaker({ open, onOpenChange, title, submitLabel, onSubmit }: StampMakerProps) {
  const { t } = useTranslation("stamps");
  const [step, setStep] = useState<Step>("pick");
  const [file, setFile] = useState<File | null>(null);
  const [cutout, setCutout] = useState<Blob | null>(null);
  const [meta, setMeta] = useState<StampMeta>(EMPTY_META);
  const [progress, setProgress] = useState<number | null>(null);
  const preview = useObjectUrl(cutout);

  // Every opening starts from scratch.
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setStep("pick");
      setFile(null);
      setCutout(null);
      setMeta(EMPTY_META);
      setProgress(null);
    }
  }

  const pick = (picked: File | undefined) => {
    if (!picked) return;
    if (!picked.type.startsWith("image/")) {
      toast.error(t("editor.notAnImage"));
      return;
    }
    setFile(picked);
    setMeta({ ...EMPTY_META, name: nameFromFile(picked.name) });
    setStep("edit");
  };

  const submit = async () => {
    if (!cutout) return;
    setProgress(0);
    try {
      const extension = cutout.type === "image/webp" ? "webp" : "png";
      const upload = new File([cutout], `stamp.${extension}`, { type: cutout.type });
      // The upload is most of the wait; processing the small image takes a moment more.
      const asset = await uploadAndProcess(upload, {
        purpose: media.AssetPurpose.STAMP,
        onProgress: (fraction) => setProgress(fraction * 0.9),
      });
      await onSubmit({
        assetId: asset.id,
        name: meta.name.trim(),
        emoji: meta.emoji,
        keywords: parseKeywords(meta.keywords),
      });
      onOpenChange(false);
    } catch (error) {
      toast.error(uploadErrorMessage(error));
    } finally {
      setProgress(null);
    }
  };

  const busy = progress !== null;
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => !busy && onOpenChange(next)}
      title={
        step === "pick" ? t("editor.pickTitle") : step === "edit" ? t("editor.editTitle") : title
      }
      description={step === "pick" ? t("editor.pickLead") : undefined}
      className="sm:w-[min(34rem,calc(100vw-2rem))]"
    >
      {step === "pick" && <DropZone onPick={pick} />}
      {/* The editor stays mounted behind the last step, so "back" returns to the work as it was. */}
      {file && step !== "pick" && (
        <div className={cn(step !== "edit" && "hidden")}>
          <CutoutEditor
            file={file}
            onBack={() => setStep("pick")}
            onDone={(image) => {
              setCutout(image);
              setStep("details");
            }}
          />
        </div>
      )}
      {step === "details" && (
        <form
          className="flex flex-col gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <div
            className="sticker mx-auto grid size-40 place-items-center rounded-[1.75rem] bg-surface"
            style={{
              backgroundImage:
                "repeating-conic-gradient(var(--surface-3) 0% 25%, transparent 0% 50%)",
              backgroundSize: "20px 20px",
            }}
          >
            {preview && (
              <img
                src={preview}
                alt={t("editor.preview")}
                className="size-36 animate-pop-in object-contain"
                data-testid="cutout-preview"
              />
            )}
          </div>
          <StampMetaFields value={meta} onChange={setMeta} />
          {busy && (
            <div
              role="progressbar"
              aria-valuenow={Math.round((progress ?? 0) * 100)}
              aria-valuemin={0}
              aria-valuemax={100}
              className="h-3 overflow-hidden rounded-full border-2 border-ink bg-surface-2"
            >
              <div
                className="h-full bg-lime transition-[width] duration-200"
                style={{ width: `${Math.max(6, Math.round((progress ?? 0) * 100))}%` }}
              />
            </div>
          )}
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={() => setStep("edit")} disabled={busy}>
              {t("common:actions.back")}
            </Button>
            <Button type="submit" variant="lime" loading={busy}>
              {submitLabel}
            </Button>
          </div>
        </form>
      )}
    </Dialog>
  );
}

/** The first step: a big target to tap (opens the file chooser or the camera roll) or to drop a picture on. */
function DropZone({ onPick }: { onPick(file: File | undefined): void }) {
  const { t } = useTranslation("stamps");
  const input = useRef<HTMLInputElement>(null);
  const [over, setOver] = useState(false);

  const onDrop = (event: DragEvent) => {
    event.preventDefault();
    setOver(false);
    onPick(event.dataTransfer.files[0]);
  };

  // A picture copied elsewhere can be pasted straight in.
  useEffect(() => {
    const onPaste = (event: ClipboardEvent) => {
      const pasted = [...(event.clipboardData?.files ?? [])].find((item) =>
        item.type.startsWith("image/"),
      );
      if (pasted) onPick(pasted);
    };
    window.addEventListener("paste", onPaste);
    return () => window.removeEventListener("paste", onPaste);
  }, [onPick]);

  return (
    <div className="flex flex-col gap-3">
      <button
        type="button"
        onClick={() => input.current?.click()}
        onDragOver={(event) => {
          event.preventDefault();
          setOver(true);
        }}
        onDragLeave={() => setOver(false)}
        onDrop={onDrop}
        className={cn(
          "flex min-h-56 flex-col items-center justify-center gap-3 rounded-[1.75rem] border-[3px] border-dashed border-line bg-surface-2 p-6 text-center transition-colors hover:border-brand",
          over && "border-brand bg-brand-soft",
        )}
      >
        <span className="sticker grid size-16 animate-float place-items-center rounded-3xl bg-sun text-[#17122b]">
          <ImagePlus className="size-8" />
        </span>
        <span className="text-lg font-black">{t("editor.pick")}</span>
        <span className="text-sm text-ink-soft">{t("editor.pickHint")}</span>
      </button>
      <input
        ref={input}
        type="file"
        accept="image/*"
        className="sr-only"
        tabIndex={-1}
        aria-label={t("editor.pick")}
        onChange={(event) => onPick(event.target.files?.[0])}
      />
      <p className="px-1 text-xs leading-relaxed text-ink-faint">{t("editor.pickTip")}</p>
    </div>
  );
}

/** A temporary URL for showing a blob, released when the blob changes. */
function useObjectUrl(blob: Blob | null): string | null {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    if (!blob) {
      setUrl(null);
      return;
    }
    const created = URL.createObjectURL(blob);
    setUrl(created);
    return () => URL.revokeObjectURL(created);
  }, [blob]);
  return url;
}

/** Makes a personal stamp ("quick sticker"): cut out a picture and it is in the picker. */
export function PersonalStampMaker({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t } = useTranslation("stamps");
  const queryClient = useQueryClient();
  return (
    <StampMaker
      open={open}
      onOpenChange={onOpenChange}
      title={t("editor.personalTitle")}
      submitLabel={t("editor.personalSubmit")}
      onSubmit={async (made) => {
        await libraryClient().createPersonalStamp(made);
        toast.success(t("toast.personalCreated"));
        await refreshHoldings(queryClient);
      }}
    />
  );
}
