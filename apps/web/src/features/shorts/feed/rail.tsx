import { useNavigate } from "@tanstack/react-router";
import type { shorts } from "@tokuly/api";
import { shortSubject } from "@tokuly/core/shorts";
import {
  BarChart3,
  Bookmark,
  Columns2,
  EyeOff,
  Flag,
  Heart,
  Layers,
  Link2,
  MessageCircle,
  MessageCircleOff,
  MoreHorizontal,
  Pencil,
  Send,
  Share2,
  Shuffle,
  Upload,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { Menu, type MenuAction } from "#/components/ui/menu";
import {
  compactCount,
  ReportDialog,
  useBurst,
  useShareToChat,
  useSubjectLive,
} from "#/features/engagement";
import { cn } from "#/lib/cn";
import { useNotInterested, useSaveShort, useShareShort, useShortLike } from "../actions";
import { RemixesDialog } from "../remixes-dialog";

/** One control of the rail: an icon in a round target with its count underneath. */
function RailButton({
  label,
  count,
  children,
  className,
  ...props
}: {
  label: string;
  count?: number;
  children: ReactNode;
  className?: string;
} & React.ButtonHTMLAttributes<HTMLButtonElement>) {
  const { i18n } = useTranslation();
  return (
    <button
      type="button"
      aria-label={label}
      className={cn(
        "group flex w-12 flex-col items-center gap-0.5 text-white outline-none disabled:opacity-50",
        className,
      )}
      {...props}
    >
      <span className="relative grid size-11 place-items-center rounded-full bg-black/35 backdrop-blur-sm transition-transform group-active:scale-90 group-focus-visible:ring-[3px] group-focus-visible:ring-white">
        {children}
      </span>
      {count !== undefined && (
        <span className="text-xs font-black tabular-nums [text-shadow:0_1px_2px_rgb(0_0_0/0.7)]">
          {compactCount(count, i18n.language)}
        </span>
      )}
    </button>
  );
}

/** Keeps the short's counts live while it is the one on screen. */
function LiveCounts({ short }: { short: shorts.Short }) {
  useSubjectLive(shortSubject(short));
  return null;
}

/**
 * The column of actions on a short: like, comments, save, share and the
 * menu ("not interested", remix, report; edit and numbers for its owner).
 */
export function ActionRail({
  short,
  active,
  onComments,
  className,
}: {
  short: shorts.Short;
  /** The short is the one on screen. */
  active: boolean;
  onComments(): void;
  className?: string;
}) {
  const { t } = useTranslation("shorts");
  const navigate = useNavigate();
  const subject = shortSubject(short);
  const like = useShortLike(short);
  const save = useSaveShort(short);
  const share = useShareShort(short);
  const notInterested = useNotInterested(short);
  const toChat = useShareToChat(subject, short.caption || short.channel?.name);
  const { fire, burst } = useBurst(["💜", "✨", "💖"]);
  const [reporting, setReporting] = useState(false);
  const [remixesOpen, setRemixesOpen] = useState(false);
  const own = short.viewer?.isOwner ?? false;
  const label = short.caption || `@${short.channel?.handle ?? ""}`;

  const shareActions: MenuAction[] = [
    {
      label: t("share.chat"),
      icon: <Send />,
      onSelect: () => {
        share.recordChat();
        toChat.open();
      },
    },
    { label: t("share.copy"), icon: <Link2 />, onSelect: () => void share.copyLink() },
    ...(share.canShareOut
      ? [{ label: t("share.other"), icon: <Upload />, onSelect: () => void share.shareOut() }]
      : []),
  ];
  const remix = (kind: "remix" | "duet") =>
    void navigate({ to: "/shorts/new", search: { remix: short.shortId, kind } });
  const moreActions: MenuAction[] = [
    ...(own
      ? [
          {
            label: t("menu.edit"),
            icon: <Pencil />,
            onSelect: () =>
              void navigate({ to: "/shorts/studio/$shortId", params: { shortId: short.shortId } }),
          },
          {
            label: t("menu.analytics"),
            icon: <BarChart3 />,
            onSelect: () =>
              void navigate({ to: "/shorts/studio/analytics", search: { short: short.shortId } }),
          },
        ]
      : [
          {
            label: t("menu.notInterested"),
            icon: <EyeOff />,
            onSelect: () => void notInterested(),
          },
        ]),
    ...(short.allowRemix || own
      ? [{ label: t("menu.remix"), icon: <Shuffle />, onSelect: () => remix("remix") }]
      : []),
    ...(short.allowDuet || own
      ? [{ label: t("menu.duet"), icon: <Columns2 />, onSelect: () => remix("duet") }]
      : []),
    ...(short.remixCount > 0n
      ? [
          {
            label: t("menu.remixes", { count: Number(short.remixCount) }),
            icon: <Layers />,
            onSelect: () => setRemixesOpen(true),
          },
        ]
      : []),
    ...(own
      ? []
      : [{ label: t("menu.report"), icon: <Flag />, onSelect: () => setReporting(true) }]),
  ];

  return (
    <div className={cn("flex flex-col items-center gap-3", className)}>
      {active && <LiveCounts short={short} />}
      <RailButton
        label={t("actions.like")}
        count={like.count}
        aria-pressed={like.liked}
        disabled={!like.enabled}
        data-testid="short-like"
        onClick={() => {
          if (!like.liked) fire();
          like.toggle();
        }}
      >
        <Heart className={cn("size-6", like.liked && "fill-pink text-pink")} />
        {burst}
      </RailButton>
      <RailButton
        label={t("actions.comments")}
        count={like.comments}
        data-testid="short-comments"
        onClick={onComments}
      >
        {short.commentsEnabled ? (
          <MessageCircle className="size-6" />
        ) : (
          <MessageCircleOff className="size-6" />
        )}
      </RailButton>
      <RailButton
        label={save.saved ? t("actions.unsave") : t("actions.save")}
        count={Number(short.saveCount)}
        aria-pressed={save.saved}
        data-testid="short-save"
        onClick={save.toggle}
      >
        <Bookmark className={cn("size-6", save.saved && "fill-sun text-sun")} />
      </RailButton>
      <Menu
        trigger={
          <RailButton label={t("actions.share")} count={Number(short.shareCount)}>
            <Share2 className="size-6" />
          </RailButton>
        }
        actions={shareActions}
      />
      <Menu
        trigger={
          <RailButton label={t("actions.more")}>
            <MoreHorizontal className="size-6" />
          </RailButton>
        }
        actions={moreActions}
      />
      {toChat.sheet}
      {!own && (
        <ReportDialog
          subject={subject}
          open={reporting}
          onOpenChange={setReporting}
          label={label}
        />
      )}
      {remixesOpen && <RemixesDialog short={short} onClose={() => setRemixesOpen(false)} />}
    </div>
  );
}
