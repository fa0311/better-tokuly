import { useMutation, useQueryClient } from "@tanstack/react-query";
import { type common, media, type notes } from "@tokuly/api";
import { Check, ImagePlus, X } from "lucide-react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { TextArea, TextField } from "#/components/ui/field";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { uploadAndProcess } from "#/lib/upload";
import {
  CHANNEL_COLORS,
  CHANNEL_DESCRIPTION_MAX,
  CHANNEL_NAME_MAX,
  channelClient,
  storeChannel,
} from "./channels-api";
import { imageUrl } from "./media";

/**
 * The fields of a channel: name, description, colour and banner. Creates a
 * channel when `channel` is unset, edits it otherwise; `onSaved` gets the
 * channel as the server returned it.
 */
export function ChannelForm({
  channel,
  onSaved,
  onCancel,
}: {
  channel?: notes.NoteChannel;
  onSaved(channel: notes.NoteChannel): void;
  onCancel?(): void;
}) {
  const { t } = useTranslation("channels");
  const queryClient = useQueryClient();
  const [name, setName] = useState(channel?.name ?? "");
  const [description, setDescription] = useState(channel?.description ?? "");
  const [color, setColor] = useState(channel?.color ?? "");
  const [banner, setBanner] = useState<common.Image | undefined>(channel?.banner);
  const [uploading, setUploading] = useState(false);
  const picker = useRef<HTMLInputElement>(null);

  const save = useMutation({
    mutationFn: async () => {
      const fields = {
        name: name.trim(),
        description: description.trim(),
        color,
        bannerAssetId: banner?.assetId ?? "",
      };
      const response = channel
        ? await channelClient().updateChannel({ channelId: channel.id, ...fields })
        : await channelClient().createChannel(fields);
      return response.channel!;
    },
    onSuccess: (saved) => {
      storeChannel(queryClient, saved);
      toast.success(t(channel ? "form.saved" : "form.created"));
      onSaved(saved);
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "channels" })),
  });

  const pick = async (file: File | undefined) => {
    if (!file) return;
    setUploading(true);
    try {
      const asset = await uploadAndProcess(file, { purpose: media.AssetPurpose.NOTE_IMAGE });
      setBanner(asset.image ? { ...asset.image, assetId: asset.id } : undefined);
    } catch (error) {
      toast.error(errorMessage(error, { namespace: "channels" }));
    } finally {
      setUploading(false);
    }
  };

  const trimmed = name.trim();
  return (
    <form
      className="flex flex-col gap-4"
      onSubmit={(event) => {
        event.preventDefault();
        if (trimmed) save.mutate();
      }}
    >
      <div>
        <p className="px-1 pb-1.5 text-sm font-bold text-ink-soft">{t("form.banner")}</p>
        <div
          className="relative grid h-28 place-items-center overflow-hidden rounded-card border-2 border-line bg-surface-2"
          style={banner ? undefined : { backgroundColor: color || undefined }}
        >
          {banner && (
            <img
              src={imageUrl(banner, 640)}
              alt=""
              className="absolute inset-0 size-full object-cover"
            />
          )}
          <div className="relative flex gap-2">
            <Button
              type="button"
              variant="secondary"
              size="sm"
              icon={<ImagePlus className="size-4" />}
              loading={uploading}
              onClick={() => picker.current?.click()}
            >
              {uploading
                ? t("form.uploading")
                : t(banner ? "form.bannerChange" : "form.bannerPick")}
            </Button>
            {banner && !uploading && (
              <Button
                type="button"
                variant="secondary"
                size="sm"
                icon={<X className="size-4" />}
                onClick={() => setBanner(undefined)}
              >
                {t("form.bannerRemove")}
              </Button>
            )}
          </div>
          <input
            ref={picker}
            type="file"
            accept="image/*"
            hidden
            data-testid="channel-banner-input"
            onChange={(event) => {
              void pick(event.target.files?.[0]);
              event.target.value = "";
            }}
          />
        </div>
      </div>
      <TextField
        label={t("form.name")}
        placeholder={t("form.namePlaceholder")}
        value={name}
        maxLength={CHANNEL_NAME_MAX}
        required
        onChange={(event) => setName(event.target.value)}
      />
      <TextArea
        label={t("form.description")}
        placeholder={t("form.descriptionPlaceholder")}
        value={description}
        maxLength={CHANNEL_DESCRIPTION_MAX}
        onChange={(event) => setDescription(event.target.value)}
      />
      <fieldset>
        <legend className="px-1 pb-1.5 text-sm font-bold text-ink-soft">{t("form.color")}</legend>
        <div className="flex flex-wrap gap-2">
          <Swatch
            selected={color === ""}
            label={t("form.colorDefault")}
            onSelect={() => setColor("")}
          />
          {CHANNEL_COLORS.map((value) => (
            <Swatch
              key={value}
              color={value}
              selected={color === value}
              label={t("form.colorLabel", { color: value })}
              onSelect={() => setColor(value)}
            />
          ))}
        </div>
      </fieldset>
      <div className="flex justify-end gap-2">
        {onCancel && (
          <Button type="button" variant="ghost" onClick={onCancel} disabled={save.isPending}>
            {t("form.cancel")}
          </Button>
        )}
        <Button type="submit" loading={save.isPending} disabled={!trimmed || uploading}>
          {t(channel ? "form.save" : "form.create")}
        </Button>
      </div>
    </form>
  );
}

function Swatch({
  color,
  selected,
  label,
  onSelect,
}: {
  color?: string;
  selected: boolean;
  label: string;
  onSelect(): void;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      aria-pressed={selected}
      onClick={onSelect}
      className={cn(
        "pressable grid size-9 place-items-center rounded-full border-2",
        selected ? "border-ink" : "border-line",
        !color && "bg-brand",
      )}
      style={color ? { backgroundColor: color } : undefined}
    >
      {selected && <Check className="size-4 text-white drop-shadow" strokeWidth={3} />}
    </button>
  );
}
