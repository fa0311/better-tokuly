import { router } from "expo-router";
import { Trash2 } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Page, Row, Section } from "#/components/page";
import { Tabs } from "#/components/tabs";
import { useSettings } from "#/features/settings/data";
import { useTheme } from "#/lib/theme";
import { HistoryCalendar } from "./calendar";
import { NS } from "./data";
import { NightsList, VisitsList } from "./lists";
import { StatsPanel } from "./stats";

type Tab = "calendar" | "visits" | "nights" | "stats";

/** `/history`: the diary nobody had to write — by day, by visit, by night, in numbers. */
export function HistoryScreen() {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const [tab, setTab] = useState<Tab>("calendar");
  return (
    <Page testID="history">
      <RecordingOff />
      <Tabs<Tab>
        testID="history-tab"
        items={[
          { value: "calendar", label: t("tabs.calendar") },
          { value: "visits", label: t("tabs.visits") },
          { value: "nights", label: t("tabs.nights") },
          { value: "stats", label: t("tabs.stats") },
        ]}
        value={tab}
        onChange={setTab}
      />
      {tab === "calendar" && <HistoryCalendar />}
      {tab === "visits" && <VisitsList />}
      {tab === "nights" && <NightsList />}
      {tab === "stats" && <StatsPanel />}
      <Section>
        <Row
          testID="history-delete"
          icon={<Trash2 size={18} color={colors.danger} />}
          label={t("delete.action")}
          danger
          onPress={() => router.push("/history/delete")}
        />
      </Section>
    </Page>
  );
}

/**
 * Said on top while the account keeps no history (its privacy setting): the
 * screens are empty because nothing is recorded, not because nothing happened.
 */
function RecordingOff() {
  const { t } = useTranslation("mobile");
  const { data } = useSettings();
  if (data?.privacy?.recordHistory !== false) return null;
  return (
    <View testID="history-recording-off" className="gap-1 rounded-card bg-surface-2 p-4">
      <Text className="text-sm font-bold text-ink">{t("history.recordingOff")}</Text>
      <Pressable accessibilityRole="link" onPress={() => router.push("/settings/privacy")}>
        <Text className="text-sm font-black text-brand">{t("history.openPrivacy")}</Text>
      </Pressable>
    </View>
  );
}
