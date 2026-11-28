import { Link, type LinkProps } from "@tanstack/react-router";
import {
  BookOpenText,
  Bot,
  ChevronRight,
  KeyRound,
  Puzzle,
  ShieldCheck,
  Webhook,
} from "lucide-react";
import type { ComponentType } from "react";
import { useTranslation } from "react-i18next";
import { Page } from "#/components/ui/page";
import { cn } from "#/lib/cn";

const INK = "text-[#17122b]";

/** The parts of the developer platform, in the order people usually need them. */
const AREAS: {
  id: "tokens" | "apps" | "bots" | "webhooks" | "authorized" | "reference";
  to: NonNullable<LinkProps["to"]>;
  icon: ComponentType<{ className?: string }>;
  tint: string;
}[] = [
  { id: "tokens", to: "/developers/tokens", icon: KeyRound, tint: `bg-lime ${INK}` },
  { id: "apps", to: "/developers/apps", icon: Puzzle, tint: `bg-sky ${INK}` },
  { id: "bots", to: "/developers/bots", icon: Bot, tint: `bg-sun ${INK}` },
  { id: "webhooks", to: "/developers/webhooks", icon: Webhook, tint: `bg-coral ${INK}` },
  { id: "authorized", to: "/developers/authorized", icon: ShieldCheck, tint: `bg-mint ${INK}` },
  { id: "reference", to: "/docs/api", icon: BookOpenText, tint: "bg-brand text-on-brand" },
];

/** `/developers`: the way in to tokens, applications, bots, webhooks and the API reference. */
export function DevelopersScreen() {
  const { t } = useTranslation("dev");
  return (
    <Page title={t("title")} back="/me" wide>
      <div className="sticker rounded-card bg-brand p-5 text-on-brand">
        <p className="text-2xl leading-snug font-black">{t("home.hero")}</p>
        <p className="mt-1.5 max-w-xl text-sm leading-relaxed font-normal opacity-90">
          {t("home.lead")}
        </p>
      </div>
      <ul className="mt-4 grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {AREAS.map((area) => (
          <li key={area.id}>
            <Link
              to={area.to}
              className="pressable sticker flex h-full items-center gap-3 rounded-card bg-surface p-4"
            >
              <span
                className={cn(
                  "grid size-12 shrink-0 place-items-center rounded-2xl border-2 border-ink",
                  area.tint,
                )}
              >
                <area.icon className="size-6" />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block font-black">{t(`home.areas.${area.id}.title`)}</span>
                <span className="block text-sm leading-snug text-ink-soft">
                  {t(`home.areas.${area.id}.body`)}
                </span>
              </span>
              <ChevronRight className="size-5 shrink-0 text-ink-faint" />
            </Link>
          </li>
        ))}
      </ul>
    </Page>
  );
}
