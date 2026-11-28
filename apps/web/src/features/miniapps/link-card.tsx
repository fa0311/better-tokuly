import { type MiniAppLink, miniAppHref } from "@tokuly/core/miniapps";
import { Sparkles } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { NS, useMiniApp } from "./api";
import { AppIcon } from "./parts";

/**
 * A mini app a chat message links to: its icon and name and a button that
 * opens it with this conversation (so it can share a card back). While it
 * loads, and when it cannot be opened (not approved, taken down), it shows
 * `fallback`.
 *
 * Opening is a full page load: the mini-app page's Content-Security-Policy
 * names the approved origin as the one frame it may show, and a policy comes
 * with a page, not with a route change.
 */
export function MiniAppLinkCard({
  link,
  conversationId,
  fallback,
  tone,
}: {
  link: MiniAppLink;
  conversationId: string;
  fallback: ReactNode;
  tone: "own" | "other";
}) {
  const { t } = useTranslation(NS);
  const query = useMiniApp(link.appId);
  const miniApp = query.data?.miniApp;
  if (!miniApp) return fallback;
  const href = miniAppHref(link.appId, {
    path: link.path,
    conversationId,
    origin: miniApp.origin,
  });
  return (
    <a
      href={href}
      className={cn(
        "mt-2 flex w-72 max-w-full items-center gap-3 rounded-2xl border-2 p-3 text-left",
        tone === "own" ? "border-on-brand/30 bg-on-brand/10" : "border-line bg-surface text-ink",
      )}
      data-testid="mini-app-card"
    >
      <AppIcon icon={miniApp.icon} name={miniApp.name} />
      <span className="min-w-0 flex-1">
        <span
          className={cn(
            "flex items-center gap-1 text-[11px] font-bold",
            tone === "own" ? "text-on-brand/70" : "text-ink-faint",
          )}
        >
          <Sparkles className="size-3" />
          {t("card.label")}
        </span>
        <span className="block truncate font-black">
          <EmojiText text={miniApp.name} />
        </span>
      </span>
      <span
        className={cn(
          "rounded-full px-3 py-1 text-sm font-black",
          tone === "own" ? "bg-on-brand text-brand-strong" : "bg-brand text-on-brand",
        )}
      >
        {t("card.open")}
      </span>
    </a>
  );
}
