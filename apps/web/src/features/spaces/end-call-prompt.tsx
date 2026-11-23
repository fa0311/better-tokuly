import { useTranslation } from "react-i18next";
import { ConfirmDialog } from "#/components/ui/confirm";
import { answerCallPrompt, useCallPrompt } from "./session";

/**
 * "End the call and come in?": asked when the person enters a room or a
 * hangout while a chat call has the microphone. Mounted once for the app.
 */
export function EndCallPrompt() {
  const { t } = useTranslation("spaces");
  const open = useCallPrompt((state) => state.prompt !== null);
  return (
    <ConfirmDialog
      open={open}
      onOpenChange={(next) => !next && answerCallPrompt(false)}
      title={t("callFocus.title")}
      description={t("callFocus.body")}
      confirmLabel={t("callFocus.confirm")}
      danger
      onConfirm={() => answerCallPrompt(true)}
    />
  );
}
