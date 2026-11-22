import { Link, useNavigate } from "@tanstack/react-router";
import { ChevronRight, LogOut, Search, UserRound } from "lucide-react";
import { useTranslation } from "react-i18next";
import { HubSections } from "#/components/hub";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Page } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { openQuickSearch } from "#/features/search";
import { signOut as signOutOfApp, useMe } from "#/stores/session";

/**
 * "じぶん": the hub — who is signed in, search, and every part of the app in
 * named sections. The tiles are declared by the features themselves
 * (`src/features/<area>/hub.tsx`).
 */
export function MeScreen() {
  const { t } = useTranslation("me");
  const me = useMe();
  const navigate = useNavigate();
  if (!me?.user) return null;

  const signOut = async () => {
    await signOutOfApp();
    await navigate({ to: "/" });
  };

  return (
    <Page title={t("title")}>
      <Link
        to="/u/$handle"
        params={{ handle: me.user.handle }}
        className="pressable sticker flex items-center gap-4 rounded-card bg-surface p-4"
      >
        <Avatar user={me.user} size="lg" outlined />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-xl font-black">
            <EmojiText text={me.user.displayName} />
          </span>
          <span className="block truncate font-bold text-ink-soft">@{me.user.handle}</span>
          <span className="mt-1 inline-flex items-center gap-1 text-sm font-bold text-brand">
            <UserRound className="size-4" />
            {t("viewProfile")}
          </span>
        </span>
        <ChevronRight className="size-6 shrink-0 text-ink-faint" />
      </Link>

      {/* Search from the hub: on phones this is the way in to the quick search. */}
      <button
        type="button"
        onClick={() => openQuickSearch()}
        className="pressable sticker-sm mt-4 flex h-12 w-full items-center gap-3 rounded-full bg-surface px-4 text-left font-bold text-ink-soft"
      >
        <Search className="size-5 shrink-0" />
        <span className="min-w-0 flex-1 truncate">{t("common:hub.search")}</span>
      </button>

      <div className="mt-6">
        <HubSections variant="page" />
      </div>

      <Button
        variant="ghost"
        block
        className="mt-8 text-ink-soft"
        icon={<LogOut className="size-4" />}
        onClick={signOut}
      >
        {t("signOut")}
      </Button>
    </Page>
  );
}
