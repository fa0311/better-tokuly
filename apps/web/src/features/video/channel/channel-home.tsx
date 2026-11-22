import { Link } from "@tanstack/react-router";
import { video } from "@tokuly/api";
import { ChevronRight, Upload } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { EmptyState } from "#/components/ui/page";
import { PlaylistCard } from "../playlists/playlist-card";
import { VideoCard } from "../video-card";
import { useChannelPage } from "./channel-layout";

const { ChannelSectionKind, TrafficSource } = video;

const SECTION_KEYS: Partial<Record<video.ChannelSectionKind, string>> = {
  [ChannelSectionKind.LATEST]: "channel.sections.latest",
  [ChannelSectionKind.POPULAR]: "channel.sections.popular",
  [ChannelSectionKind.LIVE_ARCHIVES]: "channel.sections.liveArchives",
  [ChannelSectionKind.PLAYLISTS]: "channel.sections.playlists",
};

/** The channel's home tab: its featured video, then shelves of the newest and most watched videos, live archives and playlists. */
export function ChannelHome({ handle }: { handle: string }) {
  const { t } = useTranslation("video");
  const page = useChannelPage(handle);
  const channel = page.data?.channel;
  const sections = page.data?.sections ?? [];
  if (!channel) return null;

  if (sections.length === 0) {
    return channel.viewer?.isOwner ? (
      <EmptyState
        emoji="🎬"
        title={t("channel.emptyOwner")}
        body={t("channel.emptyOwnerBody")}
        action={
          <Link to="/studio/upload" className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-brand px-5 font-bold text-on-brand">
            <Upload className="size-4" />
            {t("studio.upload.action")}
          </Link>
        }
      />
    ) : (
      <EmptyState emoji="🌱" title={t("channel.empty")} body={t("channel.emptyBody")} />
    );
  }

  return (
    <div className="flex flex-col gap-8">
      {sections.map((section) => {
        if (section.kind === ChannelSectionKind.TRAILER) {
          const trailer = section.videos[0];
          return (
            trailer && (
              <section key="trailer" aria-label={t("channel.sections.trailer")} className="sticker rounded-card bg-surface p-3 sm:p-4">
                <p className="mb-2 text-xs font-black text-brand">{t("channel.sections.trailer")}</p>
                <VideoCard video={trailer} layout="row" source={TrafficSource.CHANNEL} hideChannel className="[&>a]:w-44 sm:[&>a]:w-72 sm:[&_h3]:text-lg" />
              </section>
            )
          );
        }
        const heading = SECTION_KEYS[section.kind];
        if (!heading) return null;
        if (section.kind === ChannelSectionKind.PLAYLISTS) {
          return (
            <Shelf key={section.kind} title={t(heading)} more={{ handle, tab: "playlists" }}>
              {section.playlists.map((playlist) => (
                <PlaylistCard key={playlist.id} playlist={playlist} className="w-56 shrink-0 sm:w-64" />
              ))}
            </Shelf>
          );
        }
        return (
          <Shelf key={section.kind} title={t(heading)} more={{ handle, tab: "videos" }}>
            {section.videos.map((summary) => (
              <VideoCard key={summary.id} video={summary} source={TrafficSource.CHANNEL} hideChannel className="w-60 shrink-0 sm:w-64" />
            ))}
          </Shelf>
        );
      })}
    </div>
  );
}

/** A titled row that scrolls sideways, with a way to the full list. */
function Shelf({ title, more, children }: { title: string; more: { handle: string; tab: string }; children: ReactNode }) {
  const { t } = useTranslation("video");
  return (
    <section aria-label={title}>
      <div className="mb-3 flex items-center gap-2">
        <h2 className="min-w-0 flex-1 truncate text-lg font-black">{title}</h2>
        <Link to="/c/$handle/$tab" params={more} className="inline-flex h-9 shrink-0 items-center gap-0.5 rounded-full px-3 text-sm font-bold text-ink-soft hover:bg-surface-2 hover:text-ink">
          {t("channel.seeAll")}
          <ChevronRight className="size-4" />
        </Link>
      </div>
      <div className="scrollbar-none -mx-4 flex gap-4 overflow-x-auto px-4 pb-1">{children}</div>
    </section>
  );
}
