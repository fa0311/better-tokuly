import { useMutation, useQueryClient } from "@tanstack/react-query";
import { type identity, media } from "@tokuly/api";
import { profileKeys } from "@tokuly/core/profile";
import { Camera, Trash2 } from "lucide-react";
import { type ChangeEvent, useRef, useState } from "react";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { EmojiText } from "#/features/emoji";
import { accountClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { uploadAndProcess } from "#/lib/upload";
import { useTranslation } from "#/lib/use-translation";
import { adoptMe } from "../data";
import { AVATAR_FRAME, BANNER_FRAME } from "./crop";
import { ImageCropDialog } from "./image-crop-dialog";

type Slot = "avatar" | "banner";

const SLOTS = {
  avatar: { frame: AVATAR_FRAME, purpose: media.AssetPurpose.AVATAR },
  banner: { frame: BANNER_FRAME, purpose: media.AssetPurpose.BANNER },
} as const;

/** The banner and avatar as they look on the profile, each with change / remove. */
export function ProfileImages({ me }: { me: identity.Me }) {
  const { t } = useTranslation("settings");
  const queryClient = useQueryClient();
  const [picked, setPicked] = useState<{ slot: Slot; file: File } | null>(null);
  // The slot stays set while the confirmation closes, so its text does not vanish mid-animation.
  const [removing, setRemoving] = useState<{ slot: Slot; open: boolean }>({
    slot: "avatar",
    open: false,
  });
  const askRemove = (slot: Slot) => setRemoving({ slot, open: true });
  const closeRemove = () => setRemoving((current) => ({ ...current, open: false }));
  const inputs = { avatar: useRef<HTMLInputElement>(null), banner: useRef<HTMLInputElement>(null) };

  const applied = (response: identity.UpdateProfileResponse) => {
    adoptMe(response.me);
    void queryClient.invalidateQueries({ queryKey: profileKeys.profiles });
  };

  const upload = useMutation({
    mutationFn: async ({ slot, image }: { slot: Slot; image: Blob }) => {
      const asset = await uploadAndProcess(
        new File([image], `${slot}.jpg`, { type: "image/jpeg" }),
        { purpose: SLOTS[slot].purpose },
      );
      return accountClient().updateProfile(
        slot === "avatar" ? { avatarAssetId: asset.id } : { bannerAssetId: asset.id },
      );
    },
    onSuccess: (response, { slot }) => {
      applied(response);
      setPicked(null);
      toast.success(t(`profile.${slot}.changed`));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "settings" })),
  });

  const remove = useMutation({
    // An empty asset ID clears the image.
    mutationFn: (slot: Slot) =>
      accountClient().updateProfile(
        slot === "avatar" ? { avatarAssetId: "" } : { bannerAssetId: "" },
      ),
    onSuccess: (response, slot) => {
      applied(response);
      closeRemove();
      toast.success(t(`profile.${slot}.removed`));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "settings" })),
  });

  const pick = (slot: Slot) => (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    // Reset so picking the same file again still fires a change.
    event.target.value = "";
    if (file) setPicked({ slot, file });
  };

  const user = me.user;
  const banner = me.banner?.url;
  const hasAvatar = Boolean(user?.avatar?.url);

  return (
    <div className="sticker overflow-hidden rounded-card bg-surface" data-testid="profile-images">
      {/* Without a banner the profile shows the avatar's main colour, so this does too. */}
      <div
        data-testid="banner-preview"
        className="relative aspect-[3/1] bg-cover bg-center"
        style={
          banner
            ? { backgroundImage: `url(${banner})` }
            : { background: user?.avatar?.dominantColor ?? "var(--brand)" }
        }
      >
        <div className="absolute top-3 right-3 flex gap-2">
          {banner && (
            <Button
              size="icon-sm"
              variant="secondary"
              aria-label={t("profile.banner.remove")}
              onClick={() => askRemove("banner")}
            >
              <Trash2 className="size-4" />
            </Button>
          )}
          <Button
            size="sm"
            variant="secondary"
            icon={<Camera className="size-4" />}
            onClick={() => inputs.banner.current?.click()}
          >
            {t("profile.banner.change")}
          </Button>
        </div>
      </div>
      <div className="px-4 pb-4">
        <div className="flex items-end gap-3">
          <div className="relative -mt-10 shrink-0">
            <Avatar user={user} size="xl" outlined />
            <button
              type="button"
              aria-label={t("profile.avatar.change")}
              onClick={() => inputs.avatar.current?.click()}
              className="pressable sticker-sm absolute -right-2 -bottom-2 grid size-10 place-items-center rounded-full bg-brand text-on-brand"
            >
              <Camera className="size-5" />
            </button>
          </div>
          <div className="min-w-0 flex-1 pb-0.5">
            <p className="truncate text-lg font-black">
              <EmojiText text={user?.displayName ?? ""} />
            </p>
            <p className="truncate text-sm font-bold text-ink-soft">@{user?.handle}</p>
          </div>
        </div>
        {hasAvatar && (
          <Button
            size="sm"
            variant="ghost"
            className="mt-3 -ml-2 text-ink-soft"
            icon={<Trash2 className="size-4" />}
            onClick={() => askRemove("avatar")}
          >
            {t("profile.avatar.remove")}
          </Button>
        )}
      </div>

      <input
        ref={inputs.avatar}
        type="file"
        accept="image/*"
        hidden
        aria-label={t("profile.avatar.file")}
        onChange={pick("avatar")}
      />
      <input
        ref={inputs.banner}
        type="file"
        accept="image/*"
        hidden
        aria-label={t("profile.banner.file")}
        onChange={pick("banner")}
      />

      {picked && (
        <ImageCropDialog
          key={`${picked.slot}:${picked.file.name}:${picked.file.lastModified}`}
          file={picked.file}
          frame={SLOTS[picked.slot].frame}
          title={t(`profile.${picked.slot}.cropTitle`)}
          busy={upload.isPending}
          onCancel={() => setPicked(null)}
          onCropped={(image) => upload.mutate({ slot: picked.slot, image })}
          onUnreadable={() => {
            setPicked(null);
            toast.error(t("profile.crop.unreadable"));
          }}
        />
      )}
      <ConfirmDialog
        open={removing.open}
        onOpenChange={(open) => !open && closeRemove()}
        title={t(`profile.${removing.slot}.removeTitle`)}
        description={t(`profile.${removing.slot}.removeBody`)}
        confirmLabel={t("profile.removeConfirm")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate(removing.slot)}
      />
    </div>
  );
}
