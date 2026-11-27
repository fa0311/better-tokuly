import { Link } from "@tanstack/react-router";
import { live, type video } from "@tokuly/api";
import { Check, Copy, Eye, EyeOff } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { create } from "zustand";
import { EmptyState } from "#/components/ui/page";
import { copyText } from "#/features/engagement";
import { useMyChannels } from "#/features/video";
import { cn } from "#/lib/cn";
import { maskSecret, maskUrl } from "../model";
import { ChannelFace } from "../parts";

const { StreamStatus } = live;

export const CHANNEL_KEY = "tokuly.live.studio.channel";

function storedChannel(): string {
  try {
    return window.localStorage.getItem(CHANNEL_KEY) ?? "";
  } catch {
    return "";
  }
}

/** The channel the live studio works on; remembered between visits. */
const useSelection = create<{ channelId: string }>(() => ({
  channelId: typeof window === "undefined" ? "" : storedChannel(),
}));

export function selectLiveChannel(channelId: string): void {
  useSelection.setState({ channelId });
  try {
    window.localStorage.setItem(CHANNEL_KEY, channelId);
  } catch {
    // Private windows may refuse storage; the choice then lasts for this visit.
  }
}

/** The signed-in person's channels and the one the live studio works on (their choice, else the default one). */
export function useLiveChannel() {
  const query = useMyChannels();
  const chosen = useSelection((state) => state.channelId);
  const channels = query.data?.channels ?? [];
  const channel = channels.find((candidate) => candidate.summary?.id === chosen) ?? channels[0];
  return { query, channels, channel };
}

/** Switches between the person's channels. Renders nothing for a single channel. */
export function LiveChannelPicker({
  channels,
  selectedId,
}: {
  channels: video.Channel[];
  selectedId: string | undefined;
}) {
  const { t } = useTranslation("live");
  if (channels.length < 2) return null;
  return (
    <fieldset className="scrollbar-none -mx-4 mb-4 flex min-w-0 gap-2 overflow-x-auto px-4 pb-1">
      <legend className="sr-only">{t("studio.channelPicker")}</legend>
      {channels.map((channel) => {
        const summary = channel.summary;
        if (!summary) return null;
        const active = summary.id === selectedId;
        return (
          <button
            key={summary.id}
            type="button"
            aria-pressed={active}
            onClick={() => selectLiveChannel(summary.id)}
            className={cn(
              "flex h-11 shrink-0 items-center gap-2 rounded-full border-2 py-1 pr-4 pl-1.5 text-sm font-bold",
              active
                ? "border-ink bg-surface shadow-pop-sm"
                : "border-line bg-surface text-ink-soft hover:text-ink",
            )}
          >
            <ChannelFace channel={summary} />
            <span className="max-w-32 truncate">{summary.name}</span>
          </button>
        );
      })}
    </fieldset>
  );
}

/** Streaming needs a channel: where to make one. */
export function NoChannel() {
  const { t } = useTranslation("live");
  return (
    <EmptyState
      emoji="📺"
      title={t("studio.noChannelTitle")}
      body={t("studio.noChannelBody")}
      action={
        <Link
          to="/studio/channels/new"
          className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand"
        >
          {t("studio.createChannel")}
        </Link>
      }
    />
  );
}

/** A titled block of a studio page. */
export function Section({
  title,
  lead,
  actions,
  children,
  className,
}: {
  title: string;
  lead?: string;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("sticker rounded-card bg-surface p-4 sm:p-5", className)}>
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <h2 className="text-lg font-black">{title}</h2>
          {lead && <p className="mt-0.5 text-sm leading-relaxed text-ink-soft">{lead}</p>}
        </div>
        {actions}
      </div>
      <div className="mt-4 flex flex-col gap-4">{children}</div>
    </section>
  );
}

/** Where a stream is in its life, spelled out; the colour only backs it up. */
export function StatusChip({
  status,
  encoderConnected,
  className,
}: {
  status: live.StreamStatus;
  encoderConnected?: boolean;
  className?: string;
}) {
  const { t } = useTranslation("live");
  const [key, tone] =
    status === StreamStatus.LIVE
      ? (["live", "bg-live text-white"] as const)
      : status === StreamStatus.ENDED
        ? (["ended", "bg-surface-3 text-ink-soft"] as const)
        : encoderConnected
          ? (["preview", "bg-sun text-[#17122b]"] as const)
          : (["upcoming", "bg-brand-soft text-brand-strong"] as const);
  return (
    <span
      className={cn(
        "inline-flex h-6 shrink-0 items-center rounded-full px-2.5 text-xs font-black",
        tone,
        className,
      )}
      data-testid="stream-status"
      data-status={key}
    >
      {t(`status.${key}`)}
    </span>
  );
}

/**
 * A secret the owner may look at: masked until they ask, copied without being
 * shown, and never written anywhere but this field.
 */
export function SecretField({
  label,
  value,
  kind = "secret",
  hint,
}: {
  label: string;
  value: string;
  /** A URL keeps its host readable while masked. */
  kind?: "secret" | "url";
  hint?: string;
}) {
  const { t } = useTranslation("live");
  const [shown, setShown] = useState(false);
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    await copyText(value);
    setCopied(true);
    toast.success(t("secret.copied", { label }));
    setTimeout(() => setCopied(false), 2000);
  };
  return (
    <div className="flex flex-col gap-1.5" data-testid="secret-field">
      <span className="px-1 text-sm font-bold text-ink-soft">{label}</span>
      <div className="flex items-center gap-1 rounded-control border-2 border-line bg-surface-2 py-1 pr-1 pl-3">
        <code
          className="min-w-0 flex-1 truncate font-mono text-sm select-all"
          data-testid="secret-value"
          data-shown={shown}
        >
          {shown ? value : kind === "url" ? maskUrl(value) : maskSecret(value)}
        </code>
        <button
          type="button"
          aria-pressed={shown}
          aria-label={t(shown ? "secret.hide" : "secret.show", { label })}
          onClick={() => setShown(!shown)}
          className="grid size-9 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-3 hover:text-ink"
        >
          {shown ? <EyeOff className="size-4" /> : <Eye className="size-4" />}
        </button>
        <button
          type="button"
          aria-label={t("secret.copy", { label })}
          onClick={() => void copy()}
          className="grid size-9 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-3 hover:text-ink"
        >
          {copied ? <Check className="size-4 text-success" /> : <Copy className="size-4" />}
        </button>
      </div>
      {hint && <p className="px-1 text-xs text-ink-faint">{hint}</p>}
    </div>
  );
}

/** A plain value with a copy button (addresses that are no secret). */
export function CopyField({ label, value }: { label: string; value: string }) {
  const { t } = useTranslation("live");
  return (
    <div className="flex flex-col gap-1.5">
      <span className="px-1 text-sm font-bold text-ink-soft">{label}</span>
      <div className="flex items-center gap-1 rounded-control border-2 border-line bg-surface-2 py-1 pr-1 pl-3">
        <code className="min-w-0 flex-1 truncate font-mono text-sm select-all">{value}</code>
        <button
          type="button"
          aria-label={t("secret.copy", { label })}
          onClick={() =>
            void copyText(value).then(() => toast.success(t("secret.copied", { label })))
          }
          className="grid size-9 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-3 hover:text-ink"
        >
          <Copy className="size-4" />
        </button>
      </div>
    </div>
  );
}

/** A number with its name, for dashboards. */
export function Stat({
  label,
  value,
  testId,
}: {
  label: string;
  value: ReactNode;
  testId?: string;
}) {
  return (
    <div className="rounded-2xl bg-surface-2 px-3 py-2.5" data-testid={testId}>
      <div className="text-xl font-black tabular-nums">{value}</div>
      <div className="text-xs font-bold text-ink-faint">{label}</div>
    </div>
  );
}
