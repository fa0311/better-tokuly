import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { errorReason, isNotFound, type video } from "@tokuly/api";
import { PartyPopper, Play, Users } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { EmptyState } from "#/components/ui/page";
import { PaneSpinner } from "#/components/ui/spinner";
import { errorMessage } from "#/lib/errors";
import { partyClient, videoKeys } from "../api";
import { Thumbnail } from "../video-card";
import { MemberAvatar } from "./chat";
import { PartyRoomView } from "./party-room";

/** What an invite link shows before joining: the video, who hosts, how many are in. */
export function usePartyPreview(code: string) {
  return useQuery({
    queryKey: videoKeys.partyPreview(code),
    queryFn: () => partyClient().previewParty({ code }),
    retry: false,
  });
}

/** The card of a party one is about to join; `children` is the way in (a button, or a guest's name form). */
export function PartyPreviewCard({
  preview,
  children,
}: {
  preview: video.PreviewPartyResponse;
  children: ReactNode;
}) {
  const { t } = useTranslation("video");
  const party = preview.party;
  return (
    <div
      className="sticker w-full max-w-md rounded-card bg-surface p-5 text-center"
      data-testid="party-preview"
    >
      <p className="inline-flex items-center gap-1.5 rounded-full bg-pink px-3 py-1 text-sm font-black text-white">
        <PartyPopper className="size-4" />
        {t("party.title")}
      </p>
      {party?.video ? (
        <>
          <Thumbnail video={party.video} sizes={400} className="mt-4" />
          <h1 className="mt-3 line-clamp-2 text-xl leading-snug font-black">{party.video.title}</h1>
        </>
      ) : (
        <h1 className="mt-4 text-xl font-black">{t("party.preview.hiddenVideo")}</h1>
      )}
      <p className="mt-2 flex flex-wrap items-center justify-center gap-x-3 gap-y-1 text-sm font-bold text-ink-soft">
        {preview.host && (
          <span className="inline-flex items-center gap-1.5">
            <MemberAvatar member={preview.host} size="xs" />
            {t("party.preview.host", { name: preview.host.name })}
          </span>
        )}
        <span className="inline-flex items-center gap-1">
          <Users className="size-4" />
          {t("party.watching", { count: party?.memberCount ?? 0 })}
        </span>
      </p>
      <div className="mt-5">{children}</div>
    </div>
  );
}

/** A party that is over, or was never there. */
export function PartyGone({ ended, videoId }: { ended: boolean; videoId?: string }) {
  const { t } = useTranslation("video");
  return (
    <EmptyState
      emoji={ended ? "👋" : "🫥"}
      title={ended ? t("party.ended.title") : t("party.notFound")}
      body={ended ? t("party.ended.generic") : t("party.notFoundBody")}
      action={
        videoId && (
          <Link
            to="/watch/$videoId"
            params={{ videoId }}
            search={{}}
            className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-brand px-5 font-bold text-on-brand"
          >
            <Play className="size-4" />
            {t("party.keepWatching")}
          </Link>
        )
      }
    />
  );
}

/**
 * `/party/<code>` for people with an account: the host who just started the
 * party walks straight in, someone who already has a seat (a reload) gets
 * it back, and everyone else sees what is playing before they join.
 */
export function PartyScreen({ code }: { code: string }) {
  const { t } = useTranslation("video");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [session, setSession] = useState<video.PartySession | undefined>(() =>
    queryClient.getQueryData<video.PartySession>(videoKeys.party(code)),
  );
  const [seat, setSeat] = useState(0);
  const preview = usePartyPreview(code);
  const partyId = preview.data?.party?.id ?? "";
  const open = Boolean(preview.data?.party) && !preview.data?.party?.endedAt;

  // Already seated (a reload, a second tab)? Then there is nothing to ask.
  const existing = useQuery({
    queryKey: [...videoKeys.party(code), "seat"],
    queryFn: async () => {
      try {
        return (await partyClient().getParty({ partyId })).session ?? null;
      } catch (error) {
        if (isNotFound(error) || errorReason(error) === "NOT_A_PARTY_MEMBER") return null;
        throw error;
      }
    },
    enabled: !session && open && partyId !== "",
    retry: false,
    gcTime: 0,
  });
  const join = useMutation({
    mutationFn: () => partyClient().joinParty({ code }),
    onSuccess: ({ session: joined }) => {
      setSession(joined);
      setSeat((count) => count + 1);
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "video" })),
  });

  const current = session ?? existing.data ?? undefined;
  if (current?.party && current.self) {
    return (
      <PartyRoomView
        key={`${current.self.id}:${seat}`}
        session={current}
        onRejoin={() => join.mutate()}
        onLeft={() => {
          queryClient.removeQueries({ queryKey: videoKeys.party(code) });
          const videoId = current.party?.video?.shortId;
          void (videoId
            ? navigate({ to: "/watch/$videoId", params: { videoId }, search: {} })
            : navigate({ to: "/watch" }));
        }}
      />
    );
  }
  if (preview.isPending || (open && existing.isPending)) return <PaneSpinner />;
  if (preview.isError || !preview.data?.party)
    return (
      <div className="grid h-full place-items-center">
        {preview.isError && !isNotFound(preview.error) ? (
          <EmptyState emoji="😵‍💫" title={errorMessage(preview.error, { namespace: "video" })} />
        ) : (
          <PartyGone ended={false} />
        )}
      </div>
    );
  if (!open) {
    return (
      <div className="grid h-full place-items-center">
        <PartyGone ended videoId={preview.data.party.video?.shortId} />
      </div>
    );
  }
  return (
    <div className="grid h-full place-items-center overflow-y-auto p-4 pb-32 md:pb-4">
      <PartyPreviewCard preview={preview.data}>
        <Button
          size="lg"
          block
          icon={<PartyPopper className="size-5" />}
          loading={join.isPending}
          onClick={() => join.mutate()}
        >
          {t("party.preview.join")}
        </Button>
      </PartyPreviewCard>
    </div>
  );
}
