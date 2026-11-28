import { useMutation, useQueryClient } from "@tanstack/react-query";
import { media, type moments } from "@tokuly/api";
import { momentKeys, REALMOJI_FACES, realmojiGlyph } from "@tokuly/core/moments";
import { Camera, Trash2 } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { imageUrl } from "#/features/notes";
import { errorMessage } from "#/lib/errors";
import { uploadAndProcess } from "#/lib/upload";
import { useTranslation } from "#/lib/use-translation";
import { reactionClient, useRealmojis } from "./api";
import { faceKey } from "./faces";
import { SelfieDialog } from "./selfie-dialog";

/** The caller's RealMoji library: a selfie for each of the five faces, taken, retaken or removed. */
export function RealmojiLibraryDialog({ open, onClose }: { open: boolean; onClose(): void }) {
  const { t } = useTranslation("moments");
  const queryClient = useQueryClient();
  const library = useRealmojis();
  const [taking, setTaking] = useState<moments.RealmojiEmoji | null>(null);
  const pictures = new Map(library.data?.map((entry) => [entry.emoji, entry.image]));
  const refresh = () => queryClient.invalidateQueries({ queryKey: momentKeys.realmojis });

  const save = useMutation({
    mutationFn: async ({ emoji, picture }: { emoji: moments.RealmojiEmoji; picture: File }) => {
      const asset = await uploadAndProcess(picture, { purpose: media.AssetPurpose.REALMOJI });
      await reactionClient().setRealmoji({ emoji, assetId: asset.id });
    },
    onSuccess: () => {
      void refresh();
      toast.success(t("library.saved"));
    },
    onError: (error) => toast.error(errorMessage(error)),
  });
  const remove = useMutation({
    mutationFn: (emoji: moments.RealmojiEmoji) => reactionClient().deleteRealmoji({ emoji }),
    onSuccess: () => void refresh(),
    onError: (error) => toast.error(errorMessage(error)),
  });

  return (
    <>
      <Dialog
        open={open && taking === null}
        onOpenChange={(next) => !next && onClose()}
        title={t("library.title")}
        description={t("library.lead")}
      >
        <ul className="grid grid-cols-2 gap-3 sm:grid-cols-3" data-testid="realmoji-library">
          {REALMOJI_FACES.map((emoji) => {
            const picture = pictures.get(emoji);
            const name = `${realmojiGlyph(emoji)} ${t(faceKey(emoji))}`;
            return (
              <li
                key={emoji}
                className="flex flex-col items-center gap-2 rounded-card bg-surface-2 p-3"
                aria-label={name}
              >
                <span className="relative grid size-20 place-items-center overflow-hidden rounded-full border-2 border-line bg-surface text-3xl">
                  {picture ? (
                    <img src={imageUrl(picture, 80)} alt="" className="size-full object-cover" />
                  ) : (
                    <span aria-hidden>{realmojiGlyph(emoji)}</span>
                  )}
                </span>
                <span className="text-xs font-black">{name}</span>
                <div className="flex gap-1">
                  <Button
                    size="sm"
                    variant={picture ? "secondary" : "lime"}
                    icon={<Camera className="size-4" />}
                    loading={save.isPending && save.variables?.emoji === emoji}
                    onClick={() => setTaking(emoji)}
                  >
                    {picture ? t("library.retake") : t("library.take")}
                  </Button>
                  {picture && (
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      aria-label={`${t("library.delete")}: ${name}`}
                      onClick={() => remove.mutate(emoji)}
                    >
                      <Trash2 className="size-4" />
                    </Button>
                  )}
                </div>
              </li>
            );
          })}
        </ul>
      </Dialog>
      <SelfieDialog
        emoji={taking}
        onClose={() => setTaking(null)}
        onShot={(picture) => taking !== null && save.mutate({ emoji: taking, picture })}
      />
    </>
  );
}
