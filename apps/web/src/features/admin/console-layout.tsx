import { Link, Outlet, useRouterState } from "@tanstack/react-router";
import {
  Flag,
  Gavel,
  LayoutDashboard,
  Megaphone,
  Newspaper,
  ScrollText,
  Search,
  ShieldCheck,
  SmilePlus,
  Sticker,
  Users,
  Wallet,
} from "lucide-react";
import { type ComponentType, type ReactNode, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { EmptyState } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { type StaffAccess, type StaffLevel, useStaffAccess } from "./roles";
import { ConsolePage } from "./ui/console-page";

interface ConsoleNavItem {
  to:
    | "/admin"
    | "/admin/moderation"
    | "/admin/users"
    | "/admin/flags"
    | "/admin/announcements"
    | "/admin/audit"
    | "/admin/stamps"
    | "/admin/wallet"
    | "/admin/search"
    | "/admin/news"
    | "/admin/emoji";
  /** Key under `nav.` in the `admin` namespace. */
  key: string;
  icon: ComponentType<{ className?: string }>;
  /** The access the section's API asks for. */
  needs: StaffLevel;
}

/** The console's own sections, then the review pages owned by other domains. */
export const CONSOLE_NAV: ConsoleNavItem[] = [
  { to: "/admin", key: "dashboard", icon: LayoutDashboard, needs: "staff" },
  { to: "/admin/moderation", key: "moderation", icon: Gavel, needs: "moderator" },
  { to: "/admin/users", key: "users", icon: Users, needs: "moderator" },
  { to: "/admin/flags", key: "flags", icon: Flag, needs: "admin" },
  { to: "/admin/announcements", key: "announcements", icon: Megaphone, needs: "admin" },
  { to: "/admin/audit", key: "audit", icon: ScrollText, needs: "admin" },
  { to: "/admin/stamps", key: "stamps", icon: Sticker, needs: "staff" },
  { to: "/admin/wallet", key: "wallet", icon: Wallet, needs: "staff" },
  { to: "/admin/search", key: "search", icon: Search, needs: "admin" },
  { to: "/admin/news", key: "news", icon: Newspaper, needs: "moderator" },
  { to: "/admin/emoji", key: "emoji", icon: SmilePlus, needs: "moderator" },
];

/**
 * The frame of the staff console: a side navigation on wide screens (a
 * horizontal scroller otherwise) around the section pages. Accounts without a
 * staff role get a friendly dead end; the API refuses them anyway.
 */
export function ConsoleLayout() {
  const access = useStaffAccess();
  if (!access.staff) return <NotForYou />;
  return (
    <div className="flex h-full flex-col xl:flex-row">
      <ConsoleNav access={access} />
      <div className="relative min-h-0 min-w-0 flex-1">
        <Outlet />
      </div>
    </div>
  );
}

function ConsoleNav({ access }: { access: StaffAccess }) {
  const { t } = useTranslation("admin");
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const current = useRef<HTMLAnchorElement>(null);
  const isActive = (to: string) =>
    to === "/admin"
      ? pathname === "/admin" || pathname === "/admin/"
      : pathname === to || pathname.startsWith(`${to}/`);

  // On phones the navigation scrolls sideways: keep the open section in view whenever the page changes.
  useEffect(() => {
    current.current?.scrollIntoView({ block: "nearest", inline: "center" });
  }, [pathname]);

  return (
    <nav
      aria-label={t("title")}
      className="z-10 shrink-0 border-b-2 border-line bg-surface xl:flex xl:w-52 xl:flex-col xl:border-r-2 xl:border-b-0"
    >
      <div className="hidden items-center gap-2 px-4 pt-5 pb-3 xl:flex">
        <span className="grid size-9 place-items-center rounded-xl border-2 border-ink bg-coral text-[#17122b]">
          <ShieldCheck className="size-5" />
        </span>
        <span className="min-w-0">
          <span className="block truncate text-sm font-black">{t("title")}</span>
          <span className="block truncate text-xs font-bold text-ink-faint">
            {t(
              access.admin
                ? "enums.role.admin"
                : access.moderator
                  ? "enums.role.moderator"
                  : "enums.role.staff",
            )}
          </span>
        </span>
      </div>
      <ul className="scrollbar-none flex gap-1 overflow-x-auto px-3 py-2 xl:flex-col xl:overflow-visible xl:px-2 xl:py-1">
        {CONSOLE_NAV.filter((item) => access[item.needs]).map((item) => {
          const active = isActive(item.to);
          const Icon = item.icon;
          return (
            <li key={item.to} className="shrink-0">
              <Link
                ref={active ? current : undefined}
                to={item.to}
                aria-current={active ? "page" : undefined}
                className={cn(
                  "flex h-10 items-center gap-2 rounded-full px-3.5 text-sm font-bold whitespace-nowrap transition-colors xl:rounded-xl xl:px-3",
                  active
                    ? "bg-brand text-on-brand"
                    : "text-ink-soft hover:bg-surface-2 hover:text-ink",
                )}
              >
                <Icon className="size-[18px] shrink-0" />
                {t(`nav.${item.key}`)}
              </Link>
            </li>
          );
        })}
      </ul>
    </nav>
  );
}

/** What a signed-in account without a staff role finds at `/admin`. */
function NotForYou() {
  const { t } = useTranslation("admin");
  return (
    <div className="grid h-full place-items-center overflow-y-auto pb-24">
      <EmptyState
        emoji="🙅"
        title={t("gate.title")}
        body={t("gate.body")}
        action={
          <Link
            to="/map"
            className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand"
          >
            {t("gate.back")}
          </Link>
        }
      />
    </div>
  );
}

/**
 * Wraps a section that needs more than "any staff role". Someone who reaches
 * it by URL without the role is told who can open it instead of watching every
 * request fail.
 */
export function RequireAccess({ level, children }: { level: StaffLevel; children: ReactNode }) {
  const { t } = useTranslation("admin");
  const access = useStaffAccess();
  if (access[level]) return children;
  return (
    <ConsolePage title={t("title")}>
      <EmptyState
        emoji="🔐"
        title={t(level === "admin" ? "gate.adminOnlyTitle" : "gate.moderatorOnlyTitle")}
        body={t("gate.roleBody")}
      />
    </ConsolePage>
  );
}
