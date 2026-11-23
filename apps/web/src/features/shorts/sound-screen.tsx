import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { shorts } from "@tokuly/api";
import { Music2, Plus } from "lucide-react";
import { useTranslation } from "react-i18next";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { compactCount } from "#/features/engagement";
import { ChannelName } from "#/features/video";
import { errorMessage } from "#/lib/errors";
import { discoveryClient, shortKeys } from "./api";
import { imageUrl } from "./model";
import { ShortList } from "./short-tile";

/** A sound by its ID. */
export function useSound(soundId: string | undefined) {
  return useQuery({
    queryKey: shortKeys.sound(soundId ?? ""),
    queryFn: async () => (await discoveryClient().getSound({ soundId })).sound!,
    enabled: Boolean(soundId),
    retry: false,
  });
}

/** `/sound/<id>`: a sound, the shorts set to it, and the way to make one more. */
export function SoundScreen({ soundId }: { soundId: string }) {
  const { t, i18n } = useTranslation("shorts");
  const query = useSound(soundId);
  const sound = query.data;
  const original = sound?.kind === shorts.SoundKind.ORIGINAL;
  const cover = imageUrl(sound?.cover, 96);

  return (
    <Page title={t("sound.pageTitle")} back="/shorts" wide>
      {query.isPending ? (
        <Skeleton className="h-28 rounded-card" />
      ) : !sound ? (
        <EmptyState emoji="🔇" title={errorMessage(query.error, { namespace: "shorts" })} />
      ) : (
        <>
          <section className="sticker mb-5 flex flex-wrap items-center gap-4 rounded-card bg-surface p-4">
            <span className="relative grid size-20 shrink-0 place-items-center overflow-hidden rounded-3xl border-2 border-ink bg-pink text-white">
              {cover ? (
                <img src={cover} alt="" className="absolute inset-0 size-full object-cover" />
              ) : (
                <Music2 className="size-9" />
              )}
            </span>
            <div className="min-w-0 flex-1 basis-40">
              <h2 className="text-xl font-black leading-snug" data-testid="sound-title">
                <EmojiText
                  text={
                    sound.title || t("sound.original", { name: sound.channel?.name ?? "" }).trim()
                  }
                />
              </h2>
              <p className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-1.5 text-sm font-bold text-ink-soft">
                {original ? (
                  sound.channel ? (
                    <Link
                      to="/c/$handle"
                      params={{ handle: sound.channel.handle }}
                      className="inline-flex min-w-0 hover:text-ink"
                    >
                      <ChannelName channel={sound.channel} />
                    </Link>
                  ) : (
                    t("sound.originGone")
                  )
                ) : (
                  sound.artist && <EmojiText text={sound.artist} />
                )}
              </p>
              <p className="mt-1 text-sm text-ink-soft">
                {t("sound.uses", { count: compactCount(Number(sound.useCount), i18n.language) })}
              </p>
            </div>
            <div className="flex flex-wrap gap-2">
              {sound.originShortId && (
                <Link
                  to="/shorts/$shortId"
                  params={{ shortId: sound.originShortId }}
                  className="pressable sticker inline-flex h-11 items-center rounded-full bg-surface px-5 font-bold"
                >
                  {t("sound.origin")}
                </Link>
              )}
              {sound.reusable && (
                <Link
                  to="/shorts/new"
                  search={{ sound: sound.id }}
                  className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-brand px-5 font-bold text-on-brand"
                >
                  <Plus className="size-5" />
                  {t("sound.use")}
                </Link>
              )}
            </div>
          </section>
          <ShortList
            context={{ kind: "sound", key: sound.id }}
            empty={<EmptyState emoji="🎧" title={t("sound.empty")} body={t("sound.emptyBody")} />}
          />
        </>
      )}
    </Page>
  );
}
