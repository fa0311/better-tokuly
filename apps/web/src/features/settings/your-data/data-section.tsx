import { useNavigate } from "@tanstack/react-router";
import { LogOut } from "lucide-react";
import { useState } from "react";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { useTranslation } from "#/lib/use-translation";
import { signOut as signOutOfApp, useMe } from "#/stores/session";
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
  const [busy, setBusy] = useState<"here" | "everywhere" | "browser" | null>(null);
  const [confirming, setConfirming] = useState(false);

  const signOut = async (everywhere: boolean) => {
    setBusy(everywhere ? "everywhere" : "here");
    await signOutOfApp(everywhere);
    await navigate({ to: "/" });
  };
  // A shared computer: every account of this browser, switched-away ones included.
  const signOutBrowser = async () => {
    setBusy("browser");
    await signOutOfApp(false, true);
    await navigate({ to: "/" });
  };

  return (
    <Group title={t("data.signOut.title")} lead={t("data.signOut.lead")}>
      <div className="flex flex-wrap gap-2">
        <Button
          size="sm"
          variant="secondary"
          icon={<LogOut className="size-4" />}
          loading={busy === "here"}
          disabled={busy !== null}
          onClick={() => signOut(false)}
        >
          {t("data.signOut.here")}
        </Button>
        <Button
          size="sm"
          variant="secondary"
          icon={<LogOut className="size-4" />}
          loading={busy === "browser"}
          disabled={busy !== null}
          onClick={signOutBrowser}
        >
          {t("data.signOut.browser")}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          className="text-danger"
          disabled={busy !== null}
          onClick={() => setConfirming(true)}
        >
          {t("data.signOut.everywhere")}
        </Button>
      </div>
      <p className="mt-3 text-sm leading-relaxed text-ink-soft">{t("data.signOut.browserHint")}</p>
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
