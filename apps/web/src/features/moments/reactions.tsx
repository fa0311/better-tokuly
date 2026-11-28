import { useMutation, useQueryClient } from "@tanstack/react-query";
import { type MessageInitShape, media, moments } from "@tokuly/api";
import { momentKeys, REALMOJI_FACES, realmojiGlyph, setMyReaction } from "@tokuly/core/moments";
import { Sticker } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { imageUrl } from "#/features/notes";
import { StampImage, StampPicker } from "#/features/stamps";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { uploadAndProcess } from "#/lib/upload";
import { useTranslation } from "#/lib/use-translation";
import { reactionClient, useRealmojis } from "./api";
import { faceKey } from "./faces";
import { SelfieDialog } from "./selfie-dialog";

type Reaction = MessageInitShape<typeof moments.ReactRequestSchema>["reaction"];

/** A selfie for a RealMoji, uploaded and ready. */
const uploadSelfie = async (picture: File) =>
  (await uploadAndProcess(picture, { purpose: media.AssetPurpose.REALMOJI })).id;

/**
 * Reacting to a friend's unlocked moment: one of the five RealMoji faces
 * (a face not in the library yet is taken on the way), ⚡ a selfie taken
 * just for this, or a stamp. One reaction per person; the one already
 * chosen is lit, and tapping it again takes it back.
 */
export function ReactionPicker({ moment }: { moment: moments.Moment }) {
  const { t } = useTranslation("moments");
  const queryClient = useQueryClient();
  const library = useRealmojis();
  const [taking, setTaking] = useState<moments.RealmojiEmoji | null>(null);
  const mine = moment.myReaction?.reaction;

  const react = useMutation({
    mutationFn: async (request: { reaction: Reaction; assetId?: string }) =>
      (await reactionClient().react({ momentId: moment.id, ...request })).reaction,
    onSuccess: (reaction) => setMyReaction(queryClient, moment.id, reaction),
    onError: (error) => toast.error(errorMessage(error)),
  });
  const takeBack = useMutation({
    mutationFn: () => reactionClient().removeReaction({ momentId: moment.id }),
    onSuccess: () => setMyReaction(queryClient, moment.id, undefined),
    onError: (error) => toast.error(errorMessage(error)),
  });
  const shot = useMutation({
    mutationFn: async ({ emoji, picture }: { emoji: moments.RealmojiEmoji; picture: File }) => {
      const assetId = await uploadSelfie(picture);
      if (emoji === moments.RealmojiEmoji.INSTANT) return { emoji, assetId };
      await reactionClient().setRealmoji({ emoji, assetId });
      void queryClient.invalidateQueries({ queryKey: momentKeys.realmojis });
      return { emoji, assetId: undefined };
    },
    onSuccess: ({ emoji, assetId }) =>
      react.mutate({ reaction: { case: "realmoji", value: emoji }, assetId }),
    onError: (error) => toast.error(errorMessage(error)),
  });

  const pictures = new Map(library.data?.map((entry) => [entry.emoji, entry.image]));
  const busy = react.isPending || takeBack.isPending || shot.isPending;
  const chooseFace = (emoji: moments.RealmojiEmoji) => {
    if (
      mine?.case === "realmoji" &&
      mine.value.emoji === emoji &&
      emoji !== moments.RealmojiEmoji.INSTANT
    )
      return takeBack.mutate();
    if (emoji === moments.RealmojiEmoji.INSTANT || !pictures.has(emoji)) {
      if (emoji !== moments.RealmojiEmoji.INSTANT) toast(t("react.needSelfie"));
      return setTaking(emoji);
    }
    react.mutate({ reaction: { case: "realmoji", value: emoji } });
  };

  return (
    <fieldset
      className="flex flex-wrap items-center gap-1.5"
      aria-label={t("react.label")}
      aria-busy={busy || undefined}
      data-testid="moment-react"
    >
      {[...REALMOJI_FACES, moments.RealmojiEmoji.INSTANT].map((emoji) => {
        const picture = pictures.get(emoji);
        const chosen = mine?.case === "realmoji" && mine.value.emoji === emoji;
        return (
          <button
            key={emoji}
            type="button"
            disabled={busy}
            aria-pressed={chosen}
            aria-label={`${realmojiGlyph(emoji)} ${t(faceKey(emoji))}`}
            onClick={() => chooseFace(emoji)}
            className={cn(
              "pressable relative grid size-11 place-items-center overflow-hidden rounded-full border-2 bg-surface-2 text-xl",
              chosen ? "border-brand ring-2 ring-brand" : "border-line",
            )}
          >
            {picture ? (
              <>
                <img
                  src={imageUrl(picture, 44)}
                  alt=""
                  className="absolute inset-0 size-full object-cover"
                />
                <span className="absolute -right-0.5 -bottom-0.5 text-sm" aria-hidden>
                  {realmojiGlyph(emoji)}
                </span>
              </>
            ) : (
              <span aria-hidden>{realmojiGlyph(emoji)}</span>
            )}
          </button>
        );
      })}
      <StampPicker
        onPick={(stamp) =>
          mine?.case === "stamp" && mine.value.id === stamp.id
            ? takeBack.mutate()
            : react.mutate({ reaction: { case: "stampId", value: stamp.id } })
        }
        trigger={
          <button
            type="button"
            disabled={busy}
            aria-pressed={mine?.case === "stamp"}
            aria-label={t("react.stamp")}
            className={cn(
              "pressable grid size-11 place-items-center overflow-hidden rounded-full border-2 bg-surface-2",
              mine?.case === "stamp" ? "border-brand ring-2 ring-brand" : "border-line",
            )}
          >
            {mine?.case === "stamp" ? (
              <StampImage stamp={mine.value} size={32} />
            ) : (
              <Sticker className="size-5" />
            )}
          </button>
        }
      />
      <SelfieDialog
        emoji={taking}
        onClose={() => setTaking(null)}
        onShot={(picture) => taking !== null && shot.mutate({ emoji: taking, picture })}
      />
    </fieldset>
  );
}

/** The first reactions on a moment, each as its selfie or stamp with the reactor's face. */
export function ReactionStrip({ moment }: { moment: moments.Moment }) {
  const { t } = useTranslation("moments");
  if (moment.reactionCount === 0) return null;
  return (
    <div className="flex items-center gap-2" data-testid="moment-reactions">
      <ul className="flex -space-x-2">
        {moment.reactions.slice(0, 5).map((reaction) => (
          <li key={reaction.id}>
            <ReactionFace reaction={reaction} size={36} />
          </li>
        ))}
      </ul>
      <span className="text-xs font-black text-ink-soft">
        {t("card.reactions", { count: moment.reactionCount })}
      </span>
    </div>
  );
}

/** One reaction: the RealMoji selfie (or the stamp) in a circle, the emoji in the corner. */
export function ReactionFace({
  reaction,
  size,
}: {
  reaction: moments.MomentReaction;
  size: number;
}) {
  const value = reaction.reaction;
  return (
    <span
      className="relative inline-grid place-items-center rounded-full border-2 border-surface bg-surface-2"
      style={{ width: size, height: size }}
      title={reaction.user?.displayName}
    >
      {value.case === "realmoji" && value.value.image ? (
        <img
          src={imageUrl(value.value.image, size)}
          alt={reaction.user?.displayName ?? ""}
          className="size-full rounded-full object-cover"
        />
      ) : value.case === "stamp" ? (
        <StampImage stamp={value.value} size={size - 6} />
      ) : (
        <Avatar user={reaction.user} size="sm" />
      )}
      {value.case === "realmoji" && (
        <span className="absolute -right-1 -bottom-1 text-xs" aria-hidden>
          {realmojiGlyph(value.value.emoji)}
        </span>
      )}
    </span>
  );
}
