import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Page } from "#/components/ui/page";
import type { ExitReason } from "./session";

const EMOJI: Record<ExitReason, string> = {
  removed: "🚪",
  banned: "🚫",
  ended: "👋",
  closed: "🔒",
  elsewhere: "📱",
  lost: "📡",
};

/**
 * Where someone lands who is outside without having left: removed, banned,
 * the room over, entered elsewhere, or a connection that could not be
 * brought back. It says what happened and never tries to get back in by itself.
 */
export function ExitNotice({
  reason,
  action,
}: {
  reason: ExitReason;
  /** A way back in, where there is one (not after a ban). */
  action?: ReactNode;
}) {
  const { t } = useTranslation("spaces");
  return (
    <Page title={t("title")} back="/spaces">
      <div
        role="alert"
        className="flex flex-col items-center gap-3 px-6 py-14 text-center"
        data-testid="exit-notice"
        data-reason={reason}
      >
        <div className="text-6xl">{EMOJI[reason]}</div>
        <h2 className="text-xl font-black">{t(`exit.${reason}.title`)}</h2>
        <p className="max-w-xs text-sm leading-relaxed text-ink-soft">{t(`exit.${reason}.body`)}</p>
        <div className="mt-3 flex flex-wrap justify-center gap-2">
          {action}
          <Link
            to="/spaces"
            className="pressable sticker inline-flex h-11 items-center rounded-full bg-surface px-5 text-[15px] font-bold"
          >
            {t("exit.toList")}
          </Link>
        </div>
      </div>
    </Page>
  );
}
