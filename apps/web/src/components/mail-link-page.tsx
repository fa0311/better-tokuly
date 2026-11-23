import { Link } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { AuthLayout } from "#/components/auth-layout";
import { Button } from "#/components/ui/button";
import { authClient } from "#/lib/api";

/**
 * The page a mailed link opens: confirming an address (`verify`) or moving
 * the account back to its previous one (`undo`). Nothing happens until the
 * button is pressed: mail scanners open links on their own, and the link
 * can be spent once.
 */
export function MailLinkPage({ token, kind }: { token?: string; kind: "verify" | "undo" }) {
  const { t } = useTranslation("auth");
  const [state, setState] = useState<"ready" | "working" | "done" | "reverted" | "failed">(
    token ? "ready" : "failed",
  );
  // The page is server-rendered: the button only works once the app runs.
  const [interactive, setInteractive] = useState(false);
  useEffect(() => setInteractive(true), []);

  const use = () => {
    if (!token || state !== "ready") return;
    setState("working");
    authClient()
      .verifyEmail({ token })
      .then((response) => setState(response.emailChangeReverted ? "reverted" : "done"))
      .catch(() => setState("failed"));
  };

  const title = {
    ready: kind === "undo" ? t("verify.undoAsk") : t("verify.ask"),
    working: kind === "undo" ? t("verify.undoWorking") : t("verify.working"),
    done: t("verify.done"),
    reverted: t("verify.reverted"),
    failed: t("verify.failed"),
  }[state];

  return (
    <AuthLayout title={title}>
      <div className="flex flex-col items-center gap-5 py-2 text-center">
        <div className="text-6xl" aria-hidden>
          {state === "done" ? "🎉" : state === "reverted" ? "🛡️" : state === "failed" ? "😢" : "✉️"}
        </div>
        {(state === "ready" || state === "working") && (
          <>
            {kind === "undo" && (
              <p className="text-sm leading-relaxed text-ink-soft">{t("verify.undoLead")}</p>
            )}
            <Button size="lg" loading={state === "working"} disabled={!interactive} onClick={use}>
              {kind === "undo" ? t("verify.undoConfirm") : t("verify.confirm")}
            </Button>
          </>
        )}
        {state === "reverted" && (
          <>
            <p className="text-sm leading-relaxed text-ink-soft">{t("verify.revertedLead")}</p>
            <Link
              to="/reset-password"
              className="pressable sticker inline-flex h-12 items-center rounded-full bg-brand px-6 font-black text-on-brand"
            >
              {t("verify.toReset")}
            </Link>
          </>
        )}
        {(state === "done" || state === "failed") && (
          <Link
            to="/map"
            className="pressable sticker inline-flex h-12 items-center rounded-full bg-brand px-6 font-black text-on-brand"
          >
            {t("verify.toApp")}
          </Link>
        )}
      </div>
    </AuthLayout>
  );
}
