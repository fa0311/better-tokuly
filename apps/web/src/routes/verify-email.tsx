import { createFileRoute, Link } from "@tanstack/react-router";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { AuthLayout } from "#/components/auth-layout";
import { Spinner } from "#/components/ui/spinner";
import { authClient } from "#/lib/api";

export const Route = createFileRoute("/verify-email")({
  validateSearch: (search): { token?: string } => ({
    token: typeof search.token === "string" ? search.token : undefined,
  }),
  component: VerifyEmail,
});

function VerifyEmail() {
  const { t } = useTranslation("auth");
  const { token } = Route.useSearch();
  // "reverted": the link undid an address change (and signed out every device).
  const [state, setState] = useState<"working" | "done" | "reverted" | "failed">("working");
  // The token can be spent once. React runs effects twice in development, and
  // a second request would turn a successful verification into "failed".
  const requested = useRef<string | undefined>(undefined);

  useEffect(() => {
    if (!token) {
      setState("failed");
      return;
    }
    if (requested.current === token) return;
    requested.current = token;
    authClient()
      .verifyEmail({ token })
      .then((response) => setState(response.emailChangeReverted ? "reverted" : "done"))
      .catch(() => setState("failed"));
  }, [token]);

  return (
    <AuthLayout
      title={
        state === "working"
          ? t("verify.working")
          : state === "done"
            ? t("verify.done")
            : state === "reverted"
              ? t("verify.reverted")
              : t("verify.failed")
      }
    >
      <div className="flex flex-col items-center gap-5 py-2 text-center">
        {state === "working" ? (
          <Spinner className="size-8 text-brand" />
        ) : (
          <div className="text-6xl">
            {state === "done" ? "🎉" : state === "reverted" ? "🛡️" : "😢"}
          </div>
        )}
        {state === "reverted" ? (
          <>
            <p className="text-sm leading-relaxed text-ink-soft">{t("verify.revertedLead")}</p>
            <Link
              to="/reset-password"
              className="pressable sticker inline-flex h-12 items-center rounded-full bg-brand px-6 font-black text-on-brand"
            >
              {t("verify.toReset")}
            </Link>
          </>
        ) : (
          state !== "working" && (
            <Link
              to="/map"
              className="pressable sticker inline-flex h-12 items-center rounded-full bg-brand px-6 font-black text-on-brand"
            >
              {t("verify.toApp")}
            </Link>
          )
        )}
      </div>
    </AuthLayout>
  );
}
