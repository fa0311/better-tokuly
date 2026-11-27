import { Link, useNavigate } from "@tanstack/react-router";
import type { notes } from "@tokuly/api";
import { Plus, Search } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { useMe } from "#/stores/session";
import { type ChannelTab, useChannels } from "./channels-api";
import { ChannelDot } from "./channels-chip";
import { ChannelForm } from "./channels-form";
import { imageUrl } from "./media";

/** The channel directory: featured, followed and managed channels, searchable by name. */
export function ChannelsScreen({
  tab = "featured",
  query = "",
  onChange,
}: {
  tab?: ChannelTab;
  query?: string;
  onChange(next: { tab?: ChannelTab; q?: string }): void;
}) {
  const { t } = useTranslation("channels");
  const me = useMe();
  const navigate = useNavigate();
  const [typed, setTyped] = useState(query);
  const [creating, setCreating] = useState(false);
  const signedIn = Boolean(me?.user);
  const shown: ChannelTab = signedIn ? tab : "featured";
  const channels = useChannels(shown, query.trim());

  // The list follows what is typed, a moment after typing stops.
  useEffect(() => {
    if (typed === query) return;
    const timer = setTimeout(() => onChange({ tab, q: typed.trim() || undefined }), 300);
    return () => clearTimeout(timer);
  }, [typed, query, tab, onChange]);

  const rows = channels.data?.pages.flatMap((page) => page.channels) ?? [];
  return (
    <Page
      title={t("title")}
      back="/notes"
      actions={
        signedIn && (
          <Button size="sm" icon={<Plus className="size-4" />} onClick={() => setCreating(true)}>
            {t("create")}
          </Button>
        )
      }
    >
      <p className="mb-3 text-sm font-bold text-ink-soft">{t("lead")}</p>
      <TextField
        type="search"
        aria-label={t("search")}
        placeholder={t("search")}
        value={typed}
        onChange={(event) => setTyped(event.target.value)}
        trailing={<Search className="mr-2 size-4 text-ink-faint" />}
      />
      {signedIn && (
        <Tabs
          className="mt-3"
          value={shown}
          onChange={(next) =>
            onChange({ tab: next === "featured" ? undefined : next, q: query || undefined })
          }
          items={[
            { value: "featured", label: t("tabs.featured") },
            { value: "followed", label: t("tabs.followed") },
            { value: "managed", label: t("tabs.managed") },
          ]}
        />
      )}
      <div className="mt-4 flex flex-col gap-3">
        {channels.isPending &&
          Array.from({ length: 3 }, (_, index) => (
            <Skeleton key={index} className="h-24 rounded-card" />
          ))}
        {!channels.isPending && rows.length === 0 && (
          <EmptyState
            emoji="📺"
            title={query ? t("empty.search", { query }) : t(`empty.${shown}`)}
            body={query ? undefined : t(`empty.${shown}Body`)}
            action={
              signedIn && shown !== "followed" && !query ? (
                <Button onClick={() => setCreating(true)}>{t("create")}</Button>
              ) : undefined
            }
          />
        )}
        {rows.map((channel) => (
          <ChannelRow key={channel.id} channel={channel} />
        ))}
        <LoadMore query={channels} />
      </div>
      <Dialog open={creating} onOpenChange={setCreating} title={t("form.createTitle")}>
        <ChannelForm
          onCancel={() => setCreating(false)}
          onSaved={(channel) => {
            setCreating(false);
            void navigate({ to: "/notes/channels/$channelId", params: { channelId: channel.id } });
          }}
        />
      </Dialog>
    </Page>
  );
}

/** A channel in a list: banner strip, name, description and numbers. */
export function ChannelRow({ channel }: { channel: notes.NoteChannel }) {
  const { t } = useTranslation("channels");
  return (
    <Link
      to="/notes/channels/$channelId"
      params={{ channelId: channel.id }}
      className="sticker pressable block overflow-hidden rounded-card bg-surface"
    >
      <div
        className="h-12 bg-surface-2 bg-cover bg-center"
        style={{
          backgroundColor: channel.color || undefined,
          backgroundImage: channel.banner ? `url(${imageUrl(channel.banner, 640)})` : undefined,
        }}
      />
      <div className="px-4 py-3">
        <p className="flex items-center gap-2 font-black">
          <ChannelDot color={channel.color} />
          <span className="truncate">{channel.name}</span>
          {channel.archived && (
            <span className="shrink-0 rounded-full bg-surface-3 px-2 text-[11px] text-ink-soft">
              {t("archived")}
            </span>
          )}
        </p>
        {channel.description && (
          <p className="mt-1 line-clamp-2 text-sm text-ink-soft">{channel.description}</p>
        )}
        <p className="mt-1.5 flex gap-3 text-xs font-bold text-ink-faint">
          <span>{t("followers", { count: Number(channel.followerCount) })}</span>
          <span>{t("noteCount", { count: Number(channel.noteCount) })}</span>
        </p>
      </div>
    </Link>
  );
}
