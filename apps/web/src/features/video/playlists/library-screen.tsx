import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { Plus } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { playlistClient, videoKeys } from "../api";
import { PlaylistCard } from "./playlist-card";
import { PlaylistFormDialog } from "./playlist-form";

/** `/watch/playlists`: the viewer's playlists — watch later and liked videos first — and the way to start a new one. */
export function PlaylistLibraryScreen() {
  const { t } = useTranslation("video");
  const navigate = useNavigate();
  const [creating, setCreating] = useState(false);
  const lists = useQuery({
    queryKey: videoKeys.myPlaylists(),
    queryFn: () => playlistClient().listMyPlaylists({}),
  });
  const playlists = lists.data?.playlists ?? [];

  return (
    <Page
      wide
      title={t("playlist.libraryTitle")}
      back="/watch"
      actions={
        <Button size="sm" icon={<Plus className="size-4" />} onClick={() => setCreating(true)}>
          {t("playlist.new")}
        </Button>
      }
    >
      {lists.isPending ? (
        <div className="grid grid-cols-2 gap-x-4 gap-y-6 md:grid-cols-3 lg:grid-cols-4">
          {[0, 1, 2, 3].map((card) => (
            <Skeleton key={card} className="aspect-video" />
          ))}
        </div>
      ) : lists.isError ? (
        <EmptyState emoji="😵‍💫" title={errorMessage(lists.error, { namespace: "video" })} />
      ) : (
        <>
          <div className="grid grid-cols-2 gap-x-4 gap-y-6 md:grid-cols-3 lg:grid-cols-4">
            {playlists.map((playlist) => (
              <PlaylistCard key={playlist.id} playlist={playlist} showVisibility />
            ))}
          </div>
          {playlists.length <= 2 && (
            <p className="mt-8 text-center text-sm leading-relaxed text-ink-soft">
              {t("playlist.libraryHint")}
            </p>
          )}
        </>
      )}
      <PlaylistFormDialog
        open={creating}
        onOpenChange={setCreating}
        onSaved={(playlist) => {
          setCreating(false);
          void navigate({
            to: "/playlist/$playlistId",
            params: { playlistId: playlist.shortId || playlist.id },
          });
        }}
      />
    </Page>
  );
}
