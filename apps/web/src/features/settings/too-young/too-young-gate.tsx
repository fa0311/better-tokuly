import type { identity } from "@tokuly/api";
import { isTooYoung, tooYoungUntil, watchTooYoung } from "@tokuly/core/session";
import { type ReactNode, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { AuthLayout } from "#/components/auth-layout";
import { Button } from "#/components/ui/button";
import { api } from "#/lib/api";
import { signOut, useMe } from "#/stores/session";
import { DeletionDialog, DeletionScheduled } from "../your-data/deletion-card";

/**
 * An account under 13 by the birthdate it gave sees only why it cannot use
 * Tokuly, and the way to delete itself, in place of the app (`children`).
 * The server lets such a session do nothing else. On that birthday the page
 * asks again by itself, and also when it is shown again after it.
 */
export function TooYoungGate({ children }: { children: ReactNode }) {
  const me = useMe();
  const until = tooYoungUntil(me);
  useEffect(
    () =>
      watchTooYoung(
        until,
        () => api().session.refresh(),
        (wake) => {
          const shown = () => document.visibilityState === "visible" && wake();
          document.addEventListener("visibilitychange", shown);
          window.addEventListener("online", shown);
          return () => {
            document.removeEventListener("visibilitychange", shown);
            window.removeEventListener("online", shown);
          };
        },
      ),
    [until],
  );
  if (!me || !isTooYoung(me)) return children;
  return <TooYoung me={me} />;
}

function TooYoung({ me }: { me: identity.Me }) {
  const { t } = useTranslation("settings");
  const [deleting, setDeleting] = useState(false);
  const [leaving, setLeaving] = useState(false);

  return (
    <AuthLayout title={t("tooYoung.title")} lead={t("tooYoung.lead")}>
      <div className="flex flex-col gap-4" data-testid="too-young">
        <p className="text-center text-6xl" aria-hidden>
          🎂
        </p>
        <p className="text-center text-xs text-ink-soft">{t("tooYoung.birthdate")}</p>
        {me.deletionScheduledAt ? (
          <DeletionScheduled me={me} />
        ) : (
          <>
            <p className="text-sm leading-relaxed">{t("tooYoung.deleteLead")}</p>
            <Button variant="danger" block onClick={() => setDeleting(true)}>
              {t("data.deletion.start")}
            </Button>
          </>
        )}
        <Button
          variant="ghost"
          block
          loading={leaving}
          onClick={() => {
            setLeaving(true);
            void signOut();
          }}
        >
          {t("tooYoung.signOut")}
        </Button>
      </div>
      {deleting && <DeletionDialog me={me} onClose={() => setDeleting(false)} />}
    </AuthLayout>
  );
}
