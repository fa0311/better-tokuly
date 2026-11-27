import { useMutation, useQuery } from "@tanstack/react-query";
import type { video } from "@tokuly/api";
import { Search } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { errorMessage } from "#/lib/errors";
import { feedClient, partyClient, playbackClient, playlistClient, videoClient } from "../api";
import { videoIdFromInput } from "../format";
import { Thumbnail } from "../video-card";

type Source = "related" | "playlist" | "history" | "link";

/**
 * Lets the host (or a co-host) put another video on: one related to what is
 * playing, the next from the party's playlist, something from their own
 * history, or any video by its link.
 */
export function ChangeVideoDialog({
  open,
  onOpenChange,
  party,
  onChanged,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  party: video.Party;
  onChanged(): void;
}) {
  const { t } = useTranslation("video");
  const [source, setSource] = useState<Source>(party.playlistId ? "playlist" : "related");
  const [link, setLink] = useState("");
  const currentId = party.video?.id ?? "";
  const linkId = videoIdFromInput(link);

  const related = useQuery({
    queryKey: ["video", "party", "pick", "related", currentId],
    queryFn: async () =>
      (await feedClient().listRelatedVideos({ videoId: currentId, limit: 20 })).videos,
    enabled: open && source === "related" && currentId !== "",
  });
  const playlist = useQuery({
    queryKey: ["video", "party", "pick", "playlist", party.playlistId],
    queryFn: async () =>
      (
        await playlistClient().listPlaylistItems({
          playlistId: party.playlistId,
          page: { limit: 50 },
        })
      ).items.flatMap((item) => (item.video ? [item.video] : [])),
    enabled: open && source === "playlist" && party.playlistId !== "",
  });
  const history = useQuery({
    queryKey: ["video", "party", "pick", "history"],
    queryFn: async () =>
      (await playbackClient().listWatchHistory({ page: { limit: 30 } })).items.flatMap((item) =>
        item.video ? [item.video] : [],
      ),
    enabled: open && source === "history",
  });
  const linked = useQuery({
    queryKey: ["video", "party", "pick", "link", linkId],
    queryFn: async () => (await videoClient().batchGetVideos({ videoIds: [linkId ?? ""] })).videos,
    enabled: open && source === "link" && linkId !== null,
  });

  const change = useMutation({
    mutationFn: (summary: video.VideoSummary) =>
      partyClient().setPartyVideo({ partyId: party.id, videoId: summary.id }),
    onSuccess: () => {
      onOpenChange(false);
      onChanged();
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "video" })),
  });

  const active =
    source === "related"
      ? related
      : source === "playlist"
        ? playlist
        : source === "history"
          ? history
          : linked;
  const videos = (active.data ?? []).filter((summary) => summary.id !== currentId);
  const waiting = source === "link" ? linkId !== null && linked.isPending : active.isPending;

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("party.change.title")}
      description={t("party.change.lead")}
    >
      <Tabs<Source>
        value={source}
        onChange={setSource}
        items={[
          ...(party.playlistId
            ? [{ value: "playlist" as const, label: t("party.change.playlist") }]
            : []),
          { value: "related", label: t("party.change.related") },
          { value: "history", label: t("party.change.history") },
          { value: "link", label: t("party.change.link") },
        ]}
      />
      {source === "link" && (
        <label className="mt-3 flex h-12 items-center gap-2 rounded-control border-2 border-line bg-surface px-4 focus-within:border-brand">
          <Search className="size-5 shrink-0 text-ink-faint" />
          <input
            value={link}
            onChange={(event) => setLink(event.target.value)}
            placeholder={t("party.change.linkPlaceholder")}
            aria-label={t("party.change.linkPlaceholder")}
            className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-ink-faint"
          />
        </label>
      )}
      <div className="mt-3 flex min-h-40 flex-col gap-1">
        {waiting ? (
          [0, 1, 2].map((row) => <Skeleton key={row} className="h-16" />)
        ) : videos.length === 0 ? (
          <p className="m-auto max-w-64 text-center text-sm leading-relaxed text-ink-faint">
            {source === "link"
              ? linkId
                ? t("party.change.linkNotFound")
                : t("party.change.linkHint")
              : t("party.change.empty")}
          </p>
        ) : (
          videos.map((summary) => (
            <div
              key={summary.id}
              data-testid="party-pick"
              className="flex items-center gap-3 rounded-2xl p-1.5 hover:bg-surface-2"
            >
              <Thumbnail video={summary} sizes={112} className="w-28 shrink-0 rounded-xl" />
              <div className="min-w-0 flex-1">
                <p className="line-clamp-2 text-sm leading-snug font-bold">{summary.title}</p>
                <p className="truncate text-xs text-ink-soft">{summary.channel?.name}</p>
              </div>
              <Button
                size="sm"
                variant="secondary"
                loading={change.isPending && change.variables?.id === summary.id}
                onClick={() => change.mutate(summary)}
              >
                {t("party.change.play")}
              </Button>
            </div>
          ))
        )}
      </div>
    </Dialog>
  );
}
