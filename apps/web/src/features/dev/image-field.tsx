import type { common, media } from "@tokuly/api";
import { ImagePlus, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { uploadAndProcess } from "#/lib/upload";
import { useTranslation } from "#/lib/use-translation";

/** The square picture of an application or a bot: the image, or its initial. */
export function SquareImage({
  image,
  previewUrl,
  name,
  className,
}: {
  image?: common.Image;
  /** A picture chosen in this browser that is not saved yet. */
  previewUrl?: string;
  name: string;
  className?: string;
}) {
  const url =
    previewUrl ??
    [...(image?.variants ?? [])].sort((a, b) => a.size - b.size).find((v) => v.size >= 128)?.url ??
    image?.url;
  return (
    <span
      className={cn(
        "grid size-12 shrink-0 place-items-center overflow-hidden rounded-2xl border-2 border-ink bg-sky text-xl font-black text-[#17122b]",
        className,
      )}
    >
      {url ? (
        <img src={url} alt="" className="size-full object-cover" draggable={false} />
      ) : (
        <span aria-hidden>{Array.from(name.trim() || "?")[0]?.toUpperCase()}</span>
      )}
    </span>
  );
}

/**
 * Choosing a picture for a form: uploads it at once and reports the asset.
 * `value` is `undefined` while the saved picture is kept, `""` when it was
 * taken away, and an asset ID when a new one was chosen.
 */
export function ImageField({
  label,
  name,
  current,
  purpose,
  value,
  onChange,
}: {
  label: string;
  /** Whose picture it is; its initial stands in while there is none. */
  name: string;
  current?: common.Image;
  purpose: media.AssetPurpose;
  value: string | undefined;
  onChange(assetId: string | undefined): void;
}) {
  const { t } = useTranslation("dev");
  const input = useRef<HTMLInputElement>(null);
  const [preview, setPreview] = useState<string>();
  const [busy, setBusy] = useState(false);
  useEffect(() => () => void (preview && URL.revokeObjectURL(preview)), [preview]);

  const pick = async (file: File | undefined) => {
    if (!file) return;
    setBusy(true);
    try {
      const asset = await uploadAndProcess(file, { purpose });
      setPreview(URL.createObjectURL(file));
      onChange(asset.id);
    } catch (error) {
      toast.error(errorMessage(error, { namespace: "dev" }));
    } finally {
      setBusy(false);
    }
  };
  const shown = value === "" ? undefined : current;
  return (
    <div className="flex items-center gap-3">
      <SquareImage
        image={shown}
        previewUrl={value ? preview : undefined}
        name={name}
        className="size-16"
      />
      <div className="flex flex-wrap gap-2">
        <input
          ref={input}
          type="file"
          accept="image/png,image/jpeg,image/webp,image/gif"
          className="hidden"
          aria-label={label}
          onChange={(event) => {
            void pick(event.target.files?.[0]);
            event.target.value = "";
          }}
        />
        <Button
          variant="secondary"
          size="sm"
          loading={busy}
          icon={<ImagePlus className="size-4" />}
          onClick={() => input.current?.click()}
        >
          {label}
        </Button>
        {(value || shown) && (
          <Button
            variant="ghost"
            size="sm"
            icon={<X className="size-4" />}
            onClick={() => onChange("")}
          >
            {t("image.remove")}
          </Button>
        )}
      </div>
    </div>
  );
}
