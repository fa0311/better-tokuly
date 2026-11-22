import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import type { video } from "@tokuly/api";
import { PartyPopper } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { SwitchRow } from "#/components/ui/switch";
import { errorMessage } from "#/lib/errors";
import { partyClient, videoKeys } from "../api";
import { Thumbnail } from "../video-card";

/**
 * "Watch together": starts a party for a video, with the caller as host, and
 * goes to its room. The party picks up where the host's player is.
 */
export function StartPartyDialog({
  open,
  onOpenChange,
  video: summary,
  playlistId,
  position,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  video: video.VideoSummary;
  /** The playlist being played, so the party can go on to its next video. */
  playlistId?: string;
  /** Where the host's player is right now, in seconds. */
  position(): number;
}) {
  const { t } = useTranslation("video");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [allowGuests, setAllowGuests] = useState(true);

  const start = useMutation({
    mutationFn: () =>
      partyClient().createParty({
        videoId: summary.id,
        playlistId: playlistId ?? "",
        allowGuests,
        positionSeconds: Math.min(position(), Math.max(0, summary.durationSeconds - 1)),
      }),
    onSuccess: ({ session }) => {
      const code = session?.party?.code;
      if (!session || !code) return;
      // The room opens straight away with the seat this call already gave us.
      queryClient.setQueryData(videoKeys.party(code), session);
      onOpenChange(false);
      void navigate({ to: "/party/$partyId", params: { partyId: code } });
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("party.startTitle")}
      description={t("party.startLead")}
      footer={
        <>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            {t("actions.cancel")}
          </Button>
          <Button
            icon={<PartyPopper className="size-4" />}
            loading={start.isPending}
            onClick={() => start.mutate()}
          >
            {t("party.startAction")}
          </Button>
        </>
      }
    >
      <div className="flex items-center gap-3 rounded-2xl bg-surface-2 p-2">
        <Thumbnail video={summary} sizes={112} className="w-28 shrink-0 rounded-xl" />
        <p className="line-clamp-2 min-w-0 flex-1 text-sm font-black">{summary.title}</p>
      </div>
      <div className="mt-3">
        <SwitchRow
          label={t("party.allowGuests")}
          hint={t("party.allowGuestsHint")}
          checked={allowGuests}
          onChange={setAllowGuests}
        />
      </div>
    </Dialog>
  );
}
