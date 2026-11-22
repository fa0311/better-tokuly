import { Link, useNavigate } from "@tanstack/react-router";
import { ChevronRight, LogOut } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { api } from "#/lib/api";
import { cn } from "#/lib/cn";
import { useMe } from "#/stores/session";
import { SECTIONS, type SectionId } from "./sections";

/** Who is signed in, leading to their public profile (to check how an edit looks). */
export function IdentityCard() {
  const { t } = useTranslation("settings");
  const me = useMe();
  if (!me?.user) return null;
  return (
    <Link to="/u/$handle" params={{ handle: me.user.handle }} className="pressable sticker flex items-center gap-3 rounded-card bg-surface p-3">
      <Avatar user={me.user} size="lg" outlined />
      <span className="min-w-0 flex-1">
        <span className="block truncate font-black">{me.user.displayName}</span>
        <span className="block truncate text-sm font-bold text-ink-soft">@{me.user.handle}</span>
        <span className="mt-0.5 block text-xs font-bold text-brand">{t("home.viewProfile")}</span>
      </span>
      <ChevronRight className="size-5 shrink-0 text-ink-faint" />
    </Link>
  );
}

/** The list of settings sections. `activeId` marks the one that is open next to it. */
export function SettingsNav({ activeId }: { activeId?: SectionId }) {
  const { t } = useTranslation("settings");
  const me = useMe();
  // Sections that want the person's attention get a dot.
  const attention: Partial<Record<SectionId, string>> = {};
  if (me?.email && !me.emailVerified) attention.account = t("home.attention.emailUnverified");
  if (me?.deletionScheduledAt) attention.data = t("home.attention.deletionScheduled");

  return (
    <nav aria-label={t("title")}>
      <ul className="flex flex-col gap-1">
        {SECTIONS.map((section) => {
          const Icon = section.icon;
          const active = section.id === activeId;
          const note = attention[section.id];
          const strings = section.strings ?? `sections.${section.id}`;
          return (
            <li key={section.id}>
              <Link
                to={section.to}
                aria-current={active ? "page" : undefined}
                className={cn("flex items-center gap-3 rounded-2xl px-3 py-2.5 transition-colors", active ? "bg-brand-soft" : "hover:bg-surface-2")}
              >
                <span className={cn("grid size-10 shrink-0 place-items-center rounded-xl border-2 border-ink", section.tint)}>
                  <Icon className="size-5" />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-black">{t(`${strings}.title`)}</span>
                  <span className={cn("block truncate text-sm", note ? "font-bold text-danger" : "text-ink-soft")}>{note ?? t(`${strings}.hint`)}</span>
                </span>
                {note && <span aria-hidden className="size-2.5 shrink-0 rounded-full bg-pink" />}
                <ChevronRight className="size-5 shrink-0 text-ink-faint" />
              </Link>
            </li>
          );
        })}
      </ul>
    </nav>
  );
}

/** Signs this browser out and returns to the landing page. */
export function SignOutButton({ className }: { className?: string }) {
  const { t } = useTranslation("settings");
  const navigate = useNavigate();
  const [busy, setBusy] = useState(false);
  const signOut = async () => {
    setBusy(true);
    await api().session.signOut();
    await navigate({ to: "/" });
  };
  return (
    <Button variant="ghost" block className={cn("text-ink-soft", className)} icon={<LogOut className="size-4" />} loading={busy} onClick={signOut}>
      {t("home.signOut")}
    </Button>
  );
}
