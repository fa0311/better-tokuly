import { Link, useRouterState } from "@tanstack/react-router";
import { Bell, Clapperboard, MapPinned, MessageCircle, Search, StickyNote } from "lucide-react";
import type { ComponentType, ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { CountBadge } from "#/components/ui/badge";
import { LogoMark } from "#/components/ui/logo";
import { cn } from "#/lib/cn";
import { type BadgeKey, useBadges } from "#/stores/badges";
import { useMe } from "#/stores/session";

interface NavItem {
  to: string;
  labelKey: string;
  icon: ComponentType<{ className?: string }>;
  /** Shown in the phone tab bar (the rail shows everything). */
  primary?: boolean;
  /** Unread counter to show on the icon. */
  badge?: BadgeKey;
}

const NAV: NavItem[] = [
  { to: "/map", labelKey: "nav.map", icon: MapPinned, primary: true },
  { to: "/chat", labelKey: "nav.chat", icon: MessageCircle, primary: true, badge: "chat" },
  { to: "/notes", labelKey: "nav.notes", icon: StickyNote, primary: true },
  { to: "/watch", labelKey: "nav.watch", icon: Clapperboard, primary: true },
  { to: "/search", labelKey: "nav.search", icon: Search },
  { to: "/notifications", labelKey: "nav.notifications", icon: Bell, badge: "notifications" },
];

/**
 * The signed-in frame: a rail on wide screens, a floating tab bar on phones.
 * Pages render edge to edge inside it; the map page fills the whole viewport
 * and the chrome floats above it.
 */
export function AppShell({ children }: { children: ReactNode }) {
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const isActive = (to: string) => pathname === to || pathname.startsWith(`${to}/`);

  return (
    <div className="flex h-dvh w-full overflow-hidden bg-bg">
      <Rail isActive={isActive} />
      <main className="relative min-w-0 flex-1 overflow-hidden">{children}</main>
      <TabBar isActive={isActive} />
    </div>
  );
}

function Rail({ isActive }: { isActive: (to: string) => boolean }) {
  const { t } = useTranslation("common");
  const me = useMe();
  return (
    <nav
      aria-label="main"
      className="z-30 hidden w-[76px] shrink-0 flex-col items-center gap-2 border-r-2 border-line bg-surface py-4 md:flex"
    >
      <Link to="/map" className="mb-3" aria-label="Tokuly">
        <LogoMark className="size-11" />
      </Link>
      {NAV.map((item) => (
        <RailLink key={item.to} item={item} active={isActive(item.to)} label={t(item.labelKey)} />
      ))}
      <div className="flex-1" />
      {me?.user && (
        <Link to="/me" aria-label={t("nav.me")} className="pressable rounded-2xl">
          <Avatar user={me.user} outlined />
        </Link>
      )}
    </nav>
  );
}

function RailLink({ item, active, label }: { item: NavItem; active: boolean; label: string }) {
  const Icon = item.icon;
  return (
    <Link
      to={item.to}
      aria-current={active ? "page" : undefined}
      className={cn(
        "group flex w-16 flex-col items-center gap-1 rounded-2xl py-2 text-[10px] font-bold transition-colors",
        active ? "text-brand" : "text-ink-soft hover:bg-surface-2 hover:text-ink",
      )}
    >
      <span
        className={cn(
          "relative grid size-10 place-items-center rounded-2xl transition-all",
          active && "sticker-sm bg-brand text-on-brand",
        )}
      >
        <Icon className="size-[22px]" />
        <NavBadge badge={item.badge} />
      </span>
      {label}
    </Link>
  );
}

function TabBar({ isActive }: { isActive: (to: string) => boolean }) {
  const { t } = useTranslation("common");
  const me = useMe();
  return (
    <nav
      aria-label="main"
      className="pointer-events-none fixed inset-x-0 bottom-0 z-30 flex justify-center px-4 pb-safe md:hidden"
    >
      <div className="glass sticker pointer-events-auto flex items-center gap-1 rounded-full p-1.5">
        {NAV.filter((item) => item.primary).map((item) => {
          const Icon = item.icon;
          const active = isActive(item.to);
          return (
            <Link
              key={item.to}
              to={item.to}
              aria-label={t(item.labelKey)}
              aria-current={active ? "page" : undefined}
              className={cn(
                "relative grid size-12 place-items-center rounded-full transition-all",
                active ? "bg-brand text-on-brand" : "text-ink-soft",
              )}
            >
              <Icon className="size-6" />
              <NavBadge badge={item.badge} />
            </Link>
          );
        })}
        {me?.user && (
          <Link
            to="/me"
            aria-label={t("nav.me")}
            className="relative grid size-12 place-items-center"
          >
            <Avatar user={me.user} size="sm" outlined />
            {/* Notifications have no tab of their own on phones: their count rides on "me". */}
            <NavBadge badge="notifications" />
          </Link>
        )}
      </div>
    </nav>
  );
}

/** The unread counter pinned to the corner of a navigation icon. */
function NavBadge({ badge }: { badge?: BadgeKey }) {
  const count = useBadges((badges) => (badge ? badges[badge] : 0));
  return <CountBadge count={count} className="absolute -top-1 -right-1" />;
}
