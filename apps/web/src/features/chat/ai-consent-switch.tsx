import { useTranslation } from "react-i18next";
import { SwitchRow } from "#/components/ui/switch";
import { useAssistSettings, useAssistSettingsChange } from "./ai-settings";
import { assistClient } from "./api";

/**
 * 「AI に自分のメッセージを使ってもいい」: whether the people one chats with
 * may have one's messages sent to an AI service (their reply ideas and
 * translations). Lives in the privacy settings; stored by the chat domain.
 */
export function AiMessageConsentSwitch() {
  const { t } = useTranslation("chat");
  const settings = useAssistSettings();
  const change = useAssistSettingsChange((allow: boolean) =>
    assistClient().setAiMessageConsent({ allow }),
  );
  const data = settings.data;
  if (!data) return null;
  return (
    <SwitchRow
      label={t("aiConsent.label")}
      hint={
        data.allowMessageUseIsDefault
          ? `${t("aiConsent.hint")} ${t("aiConsent.defaultNote")}`
          : t("aiConsent.hint")
      }
      checked={data.allowMessageUse}
      disabled={change.isPending}
      onChange={(allow) => change.mutate(allow)}
    />
  );
}
