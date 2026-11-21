import { Link, useNavigate } from "@tanstack/react-router";
import { Bell, ChevronRight, LogOut, Megaphone, Settings, ShieldCheck, Smile, UserRound, Users, Wallet } from "lucide-react";
import type { ComponentType } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { CountBadge } from "#/components/ui/badge";
import { Button } from "#/components/ui/button";
import { Page } from "#/components/ui/page";
import { api } from "#/lib/api";
import { cn } from "#/lib/cn";
import { type BadgeKey, useBadges } from "#/stores/badges";
import { useMe } from "#/stores/session";

interface Tile {
  to: "/friends" | "/stamps" | "/wallet" | "/notifications" | "/settings" | "/announcements" | "/admin";
  labelKey: string;
  icon: ComponentType<{ className?: string }>;
  /** Background of the icon bubble. */
  tint: string;
  badge?: BadgeKey;
}

const TILES: Tile[] = [
  { to: "/friends", labelKey: "tiles.friends", icon: Users, tint: "bg-sky", badge: "friends" },
  { to: "/notifications", labelKey: "tiles.notifications", icon: Bell, tint: "bg-pink", badge: "notifications" },
  { to: "/stamps", labelKey: "tiles.stamps", icon: Smile, tint: "bg-sun" },
  { to: "/wallet", labelKey: "tiles.wallet", icon: Wallet, tint: "bg-lime" },
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
  const count = useBadges((badges) => (tile.badge ? badges[tile.badge] : 0));
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
