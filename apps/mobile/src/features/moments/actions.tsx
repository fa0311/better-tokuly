import { common, engagement } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { Alert } from "react-native";
import { ActionSheet } from "#/components/action-sheet";
import { AccountChanged, currentAccount, ensureActive } from "#/features/session/account";
import { errorMessage } from "#/lib/errors";

const REPORT_CATEGORIES = [
  ["SPAM", engagement.ReportCategory.SPAM],
  ["HARASSMENT", engagement.ReportCategory.HARASSMENT],
  ["HATE", engagement.ReportCategory.HATE],
  ["SEXUAL", engagement.ReportCategory.SEXUAL],
  ["VIOLENCE", engagement.ReportCategory.VIOLENCE],
  ["SELF_HARM", engagement.ReportCategory.SELF_HARM],
  ["ILLEGAL", engagement.ReportCategory.ILLEGAL],
  ["OTHER", engagement.ReportCategory.OTHER],
] as const;

/** Choose a reason, confirm it, and report this Moment to moderation. */
export function MomentReportSheet({
  momentId,
  onClose,
}: {
  momentId: string | null;
  onClose(): void;
}) {
  const { t: tMoments } = useTranslation("moments");
  const { t: tMobile } = useTranslation("mobile");

  const confirm = (
    id: string,
    key: (typeof REPORT_CATEGORIES)[number][0],
    category: engagement.ReportCategory,
  ) => {
    const reason = tMobile(`ask.report.categories.${key}`);
    setTimeout(() => {
      Alert.alert(tMoments("report.confirmTitle"), tMoments("report.confirmBody", { reason }), [
        { text: tMoments("report.cancel"), style: "cancel" },
        {
          text: tMoments("report.confirmSend"),
          style: "destructive",
          onPress: () => void send(id, category),
        },
      ]);
    }, 400);
  };

  const send = async (id: string, category: engagement.ReportCategory) => {
    const account = currentAccount();
    if (!account) return;
    try {
      await account.api.service(engagement.ReportService).createReport({
        subject: { type: common.SubjectType.MOMENT, id },
        category,
        text: "",
      });
      ensureActive(account);
      Alert.alert(tMoments("report.sent"));
    } catch (error) {
      if (!(error instanceof AccountChanged))
        Alert.alert(tMoments("report.failed"), errorMessage(error));
    }
  };

  return (
    <ActionSheet
      visible={momentId !== null}
      title={tMobile("ask.report.title")}
      onClose={onClose}
      actions={REPORT_CATEGORIES.map(([key, category]) => ({
        key: `report-${key}`,
        label: tMobile(`ask.report.categories.${key}`),
        onPress: () => momentId && confirm(momentId, key, category),
      }))}
    />
  );
}
