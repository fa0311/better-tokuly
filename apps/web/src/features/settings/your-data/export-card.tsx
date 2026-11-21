import { useMutation } from "@tanstack/react-query";
import type { identity } from "@tokuly/api";
import { PackageOpen } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { accountClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { Group } from "../shell/settings-page";

/** The download link in the mail works for this long. */
const LINK_DAYS = 7;

interface ExportRequest {
  exportId: string;
  requestedAt: number;
}

/**
 * The API starts an export and mails the link when it is ready, but has no
 * way to ask how an export is doing. So this browser remembers the request
 * it made, to keep saying "on its way" after a reload.
 */
const requestKey = (userId: string) => `tokuly.exportRequest.${userId}`;

function readRequest(userId: string): ExportRequest | null {
  try {
    const parsed = JSON.parse(window.localStorage.getItem(requestKey(userId)) ?? "null") as Partial<ExportRequest> | null;
    if (!parsed || typeof parsed.exportId !== "string" || typeof parsed.requestedAt !== "number") return null;
    // Older than the link's lifetime: nothing left to wait for.
    if (Date.now() - parsed.requestedAt > LINK_DAYS * 24 * 3600 * 1000) return null;
    return { exportId: parsed.exportId, requestedAt: parsed.requestedAt };
  } catch {
    return null;
  }
}

/** Asks for an archive of everything Tokuly holds about the account. */
export function ExportCard({ me }: { me: identity.Me }) {
  const { t, i18n } = useTranslation("settings");
  const userId = me.user?.id ?? "";
  const [request, setRequest] = useState(() => readRequest(userId));

  const start = useMutation({
    mutationFn: () => accountClient().requestDataExport({}),
    onSuccess: (response) => {
      const made = { exportId: response.exportId, requestedAt: Date.now() };
      window.localStorage.setItem(requestKey(userId), JSON.stringify(made));
      setRequest(made);
      toast.success(t("data.export.requestedToast"));
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  return (
    <Group title={t("data.export.title")} lead={t("data.export.lead")}>
      {request ? (
        <div role="status" className="flex items-start gap-3 rounded-2xl bg-brand-soft p-4" data-testid="export-state">
          <PackageOpen className="mt-0.5 size-6 shrink-0 text-brand" />
          <div className="min-w-0">
            <p className="font-black">{t("data.export.requested", { date: new Date(request.requestedAt).toLocaleString(i18n.language, { dateStyle: "medium", timeStyle: "short" }) })}</p>
            <p className="mt-1 text-sm leading-relaxed text-ink-soft">{me.email ? t("data.export.requestedBody", { email: me.email, days: LINK_DAYS }) : t("data.export.requestedNoEmail")}</p>
          </div>
        </div>
      ) : (
        <p className="text-sm leading-relaxed text-ink-soft">{me.email ? t("data.export.how", { email: me.email, days: LINK_DAYS }) : t("data.export.requestedNoEmail")}</p>
      )}
      <Button size="sm" variant={request ? "secondary" : "primary"} className="mt-4" loading={start.isPending} onClick={() => start.mutate()}>
        {request ? t("data.export.again") : t("data.export.request")}
      </Button>
    </Group>
  );
}
