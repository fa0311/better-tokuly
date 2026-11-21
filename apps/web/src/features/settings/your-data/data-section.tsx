import { useNavigate } from "@tanstack/react-router";
import { LogOut } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { api } from "#/lib/api";
import { useMe } from "#/stores/session";
import { useFreshMe } from "../data";
import { Group, SettingsPage } from "../shell/settings-page";
import { DeletionCard } from "./deletion-card";
import { ExportCard } from "./export-card";

/** Taking your data with you, leaving for good, and signing out. */
export function DataSection() {
  const { t } = useTranslation("settings");
  const me = useMe();
  useFreshMe();
  if (!me) return null;
  return (
    <SettingsPage title={t("sections.data.title")}>
      <ExportCard me={me} />
      <SignOutCard />
      <DeletionCard me={me} />
    </SettingsPage>
  );
}

function SignOutCard() {
  const { t } = useTranslation("settings");
  const navigate = useNavigate();
  const [busy, setBusy] = useState<"here" | "everywhere" | null>(null);
  const [confirming, setConfirming] = useState(false);

  const signOut = async (everywhere: boolean) => {
    setBusy(everywhere ? "everywhere" : "here");
    await api().session.signOut(everywhere);
    await navigate({ to: "/" });
  };

  return (
    <Group title={t("data.signOut.title")} lead={t("data.signOut.lead")}>
      <div className="flex flex-wrap gap-2">
        <Button size="sm" variant="secondary" icon={<LogOut className="size-4" />} loading={busy === "here"} disabled={busy !== null} onClick={() => signOut(false)}>
          {t("data.signOut.here")}
        </Button>
        <Button size="sm" variant="ghost" className="text-danger" disabled={busy !== null} onClick={() => setConfirming(true)}>
          {t("data.signOut.everywhere")}
        </Button>
      </div>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t("data.signOut.everywhereTitle")}
        description={t("data.signOut.everywhereBody")}
        confirmLabel={t("data.signOut.everywhereConfirm")}
        danger
        loading={busy === "everywhere"}
        onConfirm={() => signOut(true)}
      />
    </Group>
  );
}
