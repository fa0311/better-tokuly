import { Link, useNavigate, useRouterState } from "@tanstack/react-router";
import {
  Bell,
  Clapperboard,
  LayoutGrid,
  MapPinned,
  MessageCircle,
  Search,
  StickyNote,
} from "lucide-react";
import type { ComponentType, ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Launcher, openLauncher, useLauncherOpen } from "#/components/launcher";
import { Avatar } from "#/components/ui/avatar";
import { CountBadge } from "#/components/ui/badge";
import { LogoMark } from "#/components/ui/logo";
import { openQuickSearch } from "#/features/search";
import { cn } from "#/lib/cn";
import { type PrimaryNav, primaryNavOf } from "#/seams/nav";
import { useBadge } from "#/stores/badges";
import { useMe } from "#/stores/session";
import { useTabBarHidden } from "#/stores/shell";

type Icon = ComponentType<{ className?: string }>;

/**
 * The primary destinations: one tap away on every screen. Everything else is
 * reached from the hub (`/me`, the last slot of the phone tab bar), the
 * launcher (the rail's "メニュー") and the quick search.
 *
 * Which of these a page belongs to is declared by its feature
 * (`src/features/<area>/nav.ts`, see `#/seams/nav`).
 */
const PRIMARY: {
  id: PrimaryNav;
  to: "/map" | "/chat" | "/notes" | "/watch";
  icon: Icon;
  badge?: string;
}[] = [
  { id: "map", to: "/map", icon: MapPinned },
  { id: "chat", to: "/chat", icon: MessageCircle, badge: "chat" },
  { id: "notes", to: "/notes", icon: StickyNote },
  { id: "watch", to: "/watch", icon: Clapperboard },
];

/**
 * The signed-in frame: a rail on wide screens, a floating tab bar on phones.
 * Pages render edge to edge inside it; the map page fills the whole viewport
 * and the chrome floats above it.
 *
 * A page that needs the phone's whole height (a conversation, a full-screen
 * player) calls `useHideTabBar()` from `#/stores/shell`.
 */
export function AppShell({ children }: { children: ReactNode }) {
  const current = useRouterState({ select: (state) => primaryNavOf(state.location.pathname) });
  return (
    <div className="flex h-dvh w-full overflow-hidden bg-bg">
      <Rail current={current} />
      <main className="relative min-w-0 flex-1 overflow-hidden">{children}</main>
      <TabBar current={current} />
      <Launcher />
    </div>
  );
}

function Rail({ current }: { current: PrimaryNav }) {
  const { t } = useTranslation("common");
  const me = useMe();
  const launcherOpen = useLauncherOpen();
  const navigate = useNavigate();
  return (
    <nav
      aria-label="main"
      className="z-30 hidden w-[76px] shrink-0 flex-col items-center gap-1.5 overflow-y-auto border-r-2 border-line bg-surface py-4 md:flex"
    >
      {/* Not a link: the router would mark it as the current page on the map, next to the map item. */}
      <button
        type="button"
        className="mb-3 shrink-0"
        aria-label="Tokuly"
        onClick={() => void navigate({ to: "/map" })}
      >
        <LogoMark className="size-11" />
      </button>
      {PRIMARY.map((item) => (
        <Link
          key={item.id}
          to={item.to}
          aria-current={current === item.id ? "page" : undefined}
          className={railItem(current === item.id)}
        >
          <RailIcon icon={item.icon} active={current === item.id} badge={item.badge} />
          {t(`nav.${item.id}`)}
        </Link>
      ))}
      {/* Search opens over the page that is showing; the result page has the field itself. */}
      <button
        type="button"
        onClick={() => openQuickSearch()}
        aria-current={current === "search" ? "page" : undefined}
        className={railItem(current === "search")}
      >
        <RailIcon icon={Search} active={current === "search"} />
        {t("nav.search")}
      </button>
      <Link
        to="/notifications"
        aria-current={current === "notifications" ? "page" : undefined}
        className={railItem(current === "notifications")}
      >
        <RailIcon icon={Bell} active={current === "notifications"} badge="notifications" />
        {t("nav.notifications")}
      </Link>
      <button
        type="button"
        onClick={openLauncher}
        aria-haspopup="dialog"
        aria-expanded={launcherOpen}
        className={railItem(false)}
      >
        <span className="relative grid size-10 place-items-center rounded-2xl">
          <LayoutGrid className="size-[22px]" />
        </span>
        {t("nav.more")}
      </button>
      <div className="min-h-2 flex-1" />
      {me?.user && (
        <Link
          to="/me"
          aria-label={t("nav.me")}
          aria-current={current === "me" ? "page" : undefined}
          className={cn(
            "pressable shrink-0 rounded-2xl",
            current === "me" && "ring-[3px] ring-brand ring-offset-2 ring-offset-surface",
          )}
        >
          <Avatar user={me.user} outlined />
        </Link>
      )}
    </nav>
  );
}

const railItem = (active: boolean) =>
  cn(
    "group flex w-16 shrink-0 flex-col items-center gap-1 rounded-2xl py-1.5 text-[10px] font-bold transition-colors",
    active ? "text-brand" : "text-ink-soft hover:bg-surface-2 hover:text-ink",
  );

function RailIcon({ icon: Icon, active, badge }: { icon: Icon; active: boolean; badge?: string }) {
  return (
    <span
      className={cn(
        "relative grid size-10 place-items-center rounded-2xl transition-all",
        active && "sticker-sm bg-brand text-on-brand",
      )}
    >
      <Icon className="size-[22px]" />
      <NavBadge badge={badge} />
    </span>
  );
}

function TabBar({ current }: { current: PrimaryNav }) {
  const { t } = useTranslation("common");
  const me = useMe();
  const hidden = useTabBarHidden();
  if (hidden) return null;
  // Whatever is not one of the four tabs was opened from the hub: its slot stays lit.
  const onHub = !PRIMARY.some((item) => item.id === current);
  return (
    <nav
      aria-label="main"
      className="pointer-events-none fixed inset-x-0 bottom-0 z-30 flex justify-center px-4 pb-safe md:hidden"
    >
      <div className="glass sticker pointer-events-auto flex items-center gap-1 rounded-full p-1.5">
        {PRIMARY.map((item) => {
          const active = current === item.id;
          return (
            <Link
              key={item.id}
              to={item.to}
              aria-label={t(`nav.${item.id}`)}
              aria-current={active ? "page" : undefined}
              className={cn(
                "relative grid size-12 place-items-center rounded-full transition-all",
                active ? "bg-brand text-on-brand" : "text-ink-soft",
              )}
            >
              <item.icon className="size-6" />
              <NavBadge badge={item.badge} />
            </Link>
          );
        })}
        {me?.user && (
          <Link
            to="/me"
            aria-label={t("nav.me")}
            aria-current={onHub ? "page" : undefined}
            className={cn(
              "relative grid size-12 place-items-center rounded-full transition-all",
              onHub && "bg-brand",
            )}
          >
            <Avatar user={me.user} size="sm" outlined />
            {/*
              Notifications have no tab of their own on phones: their count rides on the hub.
              (Not the sum of the hub's counters: a friend request is a notification too.)
            */}
            <NavBadge badge="notifications" />
          </Link>
        )}
      </div>
    </nav>
  );
}

/** The unread counter pinned to the corner of a navigation icon. */
function NavBadge({ badge }: { badge?: string }) {
  return <CountBadge count={useBadge(badge)} className="absolute -top-1 -right-1" />;
}
