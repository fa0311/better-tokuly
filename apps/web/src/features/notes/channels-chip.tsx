import { Link } from "@tanstack/react-router";
import type { notes } from "@tokuly/api";
import { Hash } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "#/lib/cn";

/** The dot of a channel's colour (the brand colour when it has none). */
export function ChannelDot({ color, className }: { color: string; className?: string }) {
  return (
    <span
      aria-hidden
      className={cn("inline-block size-2.5 shrink-0 rounded-full border border-ink", className)}
      style={{ backgroundColor: color || "var(--color-brand)" }}
    />
  );
}

/** 「# channel」 above a note posted into a channel: a link to the channel. */
export function ChannelChip({ channel }: { channel: notes.ChannelRef | undefined }) {
  const { t } = useTranslation("channels");
  if (!channel?.id) return null;
  return (
    <Link
      to="/notes/channels/$channelId"
      params={{ channelId: channel.id }}
      onClick={(event) => event.stopPropagation()}
      aria-label={t("chip", { name: channel.name })}
      className="mb-2 inline-flex max-w-full items-center gap-1.5 rounded-full border-2 border-line bg-surface-2 px-2.5 py-0.5 text-xs font-black text-ink-soft hover:border-ink-faint hover:text-ink"
    >
      <ChannelDot color={channel.color} />
      <Hash className="size-3 shrink-0" strokeWidth={3} />
      <span className="truncate">{channel.name}</span>
    </Link>
  );
}
