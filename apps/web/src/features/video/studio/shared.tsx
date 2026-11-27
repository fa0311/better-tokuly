import { type common, toMillis, video } from "@tokuly/api";
import { CircleAlert, Clock3, Loader } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { create } from "zustand";
import { cn } from "#/lib/cn";
import { useMyChannels } from "../api";
import { visibilityOption } from "../labels";
import { ChannelAvatar } from "../video-card";

const { ProcessingState, VideoStatus } = video;

export const CHANNEL_KEY = "tokuly.studio.channel";

/** The channel the studio is showing; remembered between visits. */
const useSelection = create<{ channelId: string }>(() => ({
  channelId: typeof window === "undefined" ? "" : (window.localStorage.getItem(CHANNEL_KEY) ?? ""),
}));

/** The studio shows the default channel again (the account that chose one left). */
export function forgetStudioChannel(): void {
  useSelection.setState({ channelId: "" });
}

export function selectStudioChannel(channelId: string): void {
  useSelection.setState({ channelId });
  window.localStorage.setItem(CHANNEL_KEY, channelId);
}

/** The signed-in person's channels and the one the studio works on (their choice, else the default one). */
export function useStudioChannel() {
  const query = useMyChannels();
  const chosen = useSelection((state) => state.channelId);
  const channels = query.data?.channels ?? [];
  const channel = channels.find((candidate) => candidate.summary?.id === chosen) ?? channels[0];
  return { query, channels, channel, maxChannels: query.data?.maxChannels ?? 5 };
}

/** Switches the studio between the person's channels. Renders nothing for a single channel. */
export function ChannelPicker({
  channels,
  selectedId,
  onSelect,
}: {
  channels: video.Channel[];
  selectedId: string | undefined;
  onSelect(channelId: string): void;
}) {
  const { t } = useTranslation("video");
  if (channels.length < 2) return null;
  return (
    <fieldset className="scrollbar-none -mx-4 flex min-w-0 gap-2 overflow-x-auto px-4 pb-1">
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
            onClick={() => onSelect(summary.id)}
            className={cn(
              "flex h-11 shrink-0 items-center gap-2 rounded-full border-2 py-1 pr-4 pl-1.5 text-sm font-bold",
              active
                ? "border-ink bg-surface shadow-pop-sm"
                : "border-line bg-surface text-ink-soft hover:text-ink",
            )}
          >
            <ChannelAvatar channel={summary} size="sm" />
            <span className="max-w-32 truncate">{summary.name}</span>
          </button>
        );
      })}
    </fieldset>
  );
}

function Pill({
  tone,
  icon,
  children,
}: {
  tone: "good" | "warn" | "bad" | "plain" | "busy";
  icon?: ReactNode;
  children: ReactNode;
}) {
  const tones = {
    good: "bg-lime text-[#17122b]",
    warn: "bg-sun text-[#17122b]",
    bad: "bg-danger text-white",
    plain: "bg-surface-2 text-ink-soft",
    busy: "bg-brand-soft text-brand-strong",
  } as const;
  return (
    <span
      className={cn(
        "inline-flex h-6 shrink-0 items-center gap-1 rounded-full px-2.5 text-xs font-black [&>svg]:size-3.5",
        tones[tone],
      )}
    >
      {icon}
      {children}
    </span>
  );
}

/**
 * Where a video stands, as the studio says it: still processing or failed,
 * then draft / scheduled / published, and — once published — who can see it.
 * Every state is spelled out; the colour only backs it up.
 */
export function VideoStateChips({
  summary,
  className,
}: {
  summary: video.VideoSummary;
  className?: string;
}) {
  const { t, i18n } = useTranslation("video");
  const visibility = visibilityOption(summary.visibility as common.Visibility);
  const VisibilityIcon = visibility.icon;
  return (
    <span
      className={cn("flex flex-wrap items-center gap-1.5", className)}
      data-testid="video-state"
    >
      {summary.processing === ProcessingState.PROCESSING && (
        <Pill tone="busy" icon={<Loader className="animate-spin" />}>
          {t("studio.state.processing")}
        </Pill>
      )}
      {summary.processing === ProcessingState.FAILED && (
        <Pill tone="bad" icon={<CircleAlert />}>
          {t("studio.state.failed")}
        </Pill>
      )}
      {summary.status === VideoStatus.DRAFT && <Pill tone="plain">{t("studio.state.draft")}</Pill>}
      {summary.status === VideoStatus.SCHEDULED && (
        <Pill tone="warn" icon={<Clock3 />}>
          {summary.scheduledAt
            ? t("studio.state.scheduledAt", {
                date: new Date(toMillis(summary.scheduledAt)).toLocaleString(i18n.language, {
                  month: "numeric",
                  day: "numeric",
                  hour: "2-digit",
                  minute: "2-digit",
                }),
              })
            : t("studio.state.scheduled")}
        </Pill>
      )}
      {summary.status === VideoStatus.PUBLISHED && (
        <Pill tone="good">{t("studio.state.published")}</Pill>
      )}
      {summary.status !== VideoStatus.DRAFT && (
        <Pill tone="plain" icon={<VisibilityIcon />}>
          {t(`visibility.${visibility.key}`)}
        </Pill>
      )}
    </span>
  );
}

/** A titled block of a studio form. */
export function FormSection({
  title,
  lead,
  children,
  className,
}: {
  title: string;
  lead?: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("sticker rounded-card bg-surface p-4 sm:p-5", className)}>
      <h2 className="text-lg font-black">{title}</h2>
      {lead && <p className="mt-0.5 text-sm leading-relaxed text-ink-soft">{lead}</p>}
      <div className="mt-4 flex flex-col gap-4">{children}</div>
    </section>
  );
}
