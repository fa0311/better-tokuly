import { media } from "@tokuly/api";
import { Camera, Users } from "lucide-react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Spinner } from "#/components/ui/spinner";
import { uploadAndProcess } from "#/lib/upload";
import { errorMessage } from "./errors";

/**
 * The round "choose a picture" button for a group's icon. Uploads the chosen
 * image and reports the asset; `imageUrl` is what the group shows now.
 */
export function GroupIconPicker({
  imageUrl,
  onUploaded,
  disabled,
}: {
  imageUrl?: string;
  onUploaded(assetId: string, previewUrl: string): void;
  disabled?: boolean;
}) {
  const { t } = useTranslation("chat");
  const input = useRef<HTMLInputElement>(null);
  const [uploading, setUploading] = useState(false);

  const choose = async (file: File | undefined) => {
    if (!file) return;
    setUploading(true);
    try {
      const asset = await uploadAndProcess(file, { purpose: media.AssetPurpose.AVATAR });
      onUploaded(asset.id, asset.image?.url || URL.createObjectURL(file));
    } catch (error) {
      toast.error(errorMessage(error));
    } finally {
      setUploading(false);
      if (input.current) input.current.value = "";
    }
  };

  return (
    <>
      <button
        type="button"
        disabled={disabled || uploading}
        onClick={() => input.current?.click()}
        aria-label={t("group.chooseIcon")}
        className="pressable sticker relative grid size-20 shrink-0 place-items-center overflow-hidden rounded-[1.4rem] bg-sky text-[#17122b] disabled:opacity-60"
      >
        {imageUrl ? (
          <img src={imageUrl} alt="" className="size-full object-cover" />
        ) : (
          <Users className="size-9" />
        )}
        <span className="absolute right-0 bottom-0 grid size-7 place-items-center rounded-tl-xl bg-ink text-surface">
          {uploading ? <Spinner className="size-4" /> : <Camera className="size-4" />}
        </span>
      </button>
      <input
        ref={input}
        type="file"
        accept="image/png,image/jpeg,image/webp,image/gif"
        className="hidden"
        data-testid="group-icon-input"
        onChange={(event) => void choose(event.target.files?.[0])}
      />
    </>
  );
}
