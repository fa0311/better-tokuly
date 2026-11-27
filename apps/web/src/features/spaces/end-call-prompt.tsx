import { lazy, Suspense, useState } from "react";
import { useTranslation } from "react-i18next";
import { answerCallPrompt, useCallPrompt } from "./session";

const ConfirmDialog = lazy(() =>
  import("#/components/ui/confirm").then((module) => ({ default: module.ConfirmDialog })),
);

/**
 * "End the call and come in?": asked when the person enters a room or a
 * hangout while a chat call has the microphone. Mounted once for the app; the
 * dialog loads the first time it is asked, and stays mounted so it can animate out.
 */
export function EndCallPrompt() {
  const { t } = useTranslation("spaces");
  const open = useCallPrompt((state) => state.prompt !== null);
  const [asked, setAsked] = useState(false);
  if (open && !asked) setAsked(true);
  if (!asked) return null;
  return (
    <Suspense fallback={null}>
      <ConfirmDialog
        open={open}
        onOpenChange={(next) => !next && answerCallPrompt(false)}
        title={t("callFocus.title")}
        description={t("callFocus.body")}
        confirmLabel={t("callFocus.confirm")}
        danger
        onConfirm={() => answerCallPrompt(true)}
      />
    </Suspense>
  );
}
