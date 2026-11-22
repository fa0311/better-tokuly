import { Link, useNavigate } from "@tanstack/react-router";
import { Bell, Bookmark, CalendarHeart, ChevronRight, Clapperboard, History, ListVideo, LogOut, MailQuestionMark, Megaphone, Newspaper, Settings, ShieldCheck, Smile, SmilePlus, UserRound, Users, Wallet } from "lucide-react";
import type { ComponentType } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { CountBadge } from "#/components/ui/badge";
import { Button } from "#/components/ui/button";
import { Page } from "#/components/ui/page";
import { SchoolHubTile } from "#/features/school";
import { useAskBadge } from "#/features/ask";
import { api } from "#/lib/api";
import { cn } from "#/lib/cn";
import { type BadgeKey, useBadges } from "#/stores/badges";
import { useMe } from "#/stores/session";

interface Tile {
  to:
    | "/friends"
    | "/plans"
    | "/ask"
    | "/stamps"
    | "/emoji"
    | "/wallet"
    | "/notifications"
    | "/settings"
    | "/announcements"
    | "/news"
    | "/admin"
    | "/bookmarks"
    | "/activity"
    | "/studio"
    | "/watch/history"
    | "/watch/playlists";
  /** A key of the `me` strings, or `<namespace>:<key>` for a tile whose feature names it. */
  labelKey: string;
  icon: ComponentType<{ className?: string }>;
  /** Background of the icon bubble. */
  tint: string;
  /** `"ask"`: unread questions and unseen flames, which the question-box feature counts itself. */
  badge?: BadgeKey | "ask";
}

const TILES: Tile[] = [
  { to: "/friends", labelKey: "tiles.friends", icon: Users, tint: "bg-sky", badge: "friends" },
  { to: "/notifications", labelKey: "tiles.notifications", icon: Bell, tint: "bg-pink", badge: "notifications" },
  { to: "/plans", labelKey: "plans:tile", icon: CalendarHeart, tint: "bg-lime" },
  { to: "/ask", labelKey: "ask:tile", icon: MailQuestionMark, tint: "bg-sun", badge: "ask" },
  { to: "/bookmarks", labelKey: "engagement:tile", icon: Bookmark, tint: "bg-sky" },
  { to: "/activity", labelKey: "notes:activityTile", icon: History, tint: "bg-brand" },
  { to: "/stamps", labelKey: "tiles.stamps", icon: Smile, tint: "bg-sun" },
  { to: "/emoji", labelKey: "emoji:tile", icon: SmilePlus, tint: "bg-mint" },
  { to: "/wallet", labelKey: "tiles.wallet", icon: Wallet, tint: "bg-lime" },
  // The news tile takes its name from the shared navigation strings.
  { to: "/news", labelKey: "common:nav.news", icon: Newspaper, tint: "bg-coral" },
  { to: "/studio", labelKey: "video:meTiles.studio", icon: Clapperboard, tint: "bg-sun" },
  { to: "/watch/history", labelKey: "video:meTiles.history", icon: History, tint: "bg-sky" },
  { to: "/watch/playlists", labelKey: "video:meTiles.playlists", icon: ListVideo, tint: "bg-pink" },
  { to: "/settings", labelKey: "tiles.settings", icon: Settings, tint: "bg-mint" },
  { to: "/announcements", labelKey: "tiles.announcements", icon: Megaphone, tint: "bg-brand-soft" },
];

const STAFF_TILE: Tile = { to: "/admin", labelKey: "tiles.admin", icon: ShieldCheck, tint: "bg-coral" };

/** "じぶん": the hub for everything about the signed-in person. */
export function MeScreen() {
  const { t } = useTranslation("me");
  const me = useMe();
  const navigate = useNavigate();
  if (!me?.user) return null;

  const isStaff = me.roles.some((role) => role === "admin" || role === "moderator" || role === "staff");
  const tiles = isStaff ? [...TILES, STAFF_TILE] : TILES;
  const signOut = async () => {
    await api().session.signOut();
    await navigate({ to: "/" });
  };

  return (
    <Page title={t("title")}>
      <Link to="/u/$handle" params={{ handle: me.user.handle }} className="pressable sticker flex items-center gap-4 rounded-card bg-surface p-4">
        <Avatar user={me.user} size="lg" outlined />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-xl font-black">{me.user.displayName}</span>
          <span className="block truncate font-bold text-ink-soft">@{me.user.handle}</span>
          <span className="mt-1 inline-flex items-center gap-1 text-sm font-bold text-brand">
            <UserRound className="size-4" />
            {t("viewProfile")}
          </span>
        </span>
        <ChevronRight className="size-6 shrink-0 text-ink-faint" />
      </Link>

      {/* School: the class that is on, or the next one. */}
      <div className="mt-3">
        <SchoolHubTile />
      </div>

      <ul className="mt-5 grid grid-cols-2 gap-3 sm:grid-cols-3">
        {tiles.map((tile) => (
          <li key={tile.to}>
            <HubTile tile={tile} label={t(tile.labelKey)} />
          </li>
        ))}
      </ul>

      <Button variant="ghost" block className="mt-8 text-ink-soft" icon={<LogOut className="size-4" />} onClick={signOut}>
        {t("signOut")}
      </Button>
    </Page>
  );
}

function HubTile({ tile, label }: { tile: Tile; label: string }) {
  const Icon = tile.icon;
  const navigationCount = useBadges((badges) => (tile.badge && tile.badge !== "ask" ? badges[tile.badge] : 0));
  const askCount = useAskBadge(tile.badge === "ask");
  const count = tile.badge === "ask" ? askCount : navigationCount;
  return (
    <Link to={tile.to} className="pressable sticker flex h-28 flex-col justify-between rounded-card bg-surface p-4">
      <span className="flex items-start justify-between">
        <span className={cn("grid size-11 place-items-center rounded-2xl border-2 border-ink text-[#17122b]", tile.tint)}>
          <Icon className="size-6" />
        </span>
        <CountBadge count={count} />
      </span>
      <span className="font-black">{label}</span>
    </Link>
  );
}
