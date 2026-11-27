import { school } from "@tokuly/api";
import { router } from "expo-router";
import { Plus } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { FlatList, RefreshControl, View } from "react-native";
import { Button } from "#/components/button";
import { EmptyState, ErrorNote, Spinner } from "#/components/page";
import { Tabs } from "#/components/tabs";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";
import { NS, useAssignments } from "./data";
import { AssignmentRow } from "./due";

type Tab = "open" | "done" | "all";

const FILTERS: Record<Tab, school.AssignmentFilter> = {
  open: school.AssignmentFilter.OPEN,
  done: school.AssignmentFilter.DONE,
  all: school.AssignmentFilter.ALL,
};

/** `/school/assignments`: the caller's assignments, still to do or done, and adding one. */
export function AssignmentsScreen() {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const [tab, setTab] = useState<Tab>("open");
  const list = useAssignments(FILTERS[tab]);
  const assignments = list.data?.assignments ?? [];
  const add = () => router.push("/school/new-assignment");
  return (
    <View className="flex-1 bg-bg" testID="school-assignments-screen">
      <View className="flex-row items-center justify-between gap-2 px-4 pt-3 pb-2">
        <Tabs
          testID="assignments-tab"
          items={[
            { value: "open", label: t("assignments.tabs.open") },
            { value: "done", label: t("assignments.tabs.done") },
            { value: "all", label: t("assignments.tabs.all") },
          ]}
          value={tab}
          onChange={setTab}
        />
        <Button
          testID="assignment-add"
          size="sm"
          icon={<Plus size={16} color={colors["on-brand"]} />}
          onPress={add}
        >
          {t("assignments.addAction")}
        </Button>
      </View>
      {list.isPending ? (
        <Spinner />
      ) : list.error && !list.data ? (
        <View className="px-4">
          <ErrorNote>{errorMessage(list.error, { namespace: NS })}</ErrorNote>
        </View>
      ) : (
        <FlatList
          testID="assignments"
          data={assignments}
          keyExtractor={(item) => item.id}
          renderItem={({ item }) => <AssignmentRow assignment={item} />}
          onEndReached={() => {
            if (list.hasNextPage && !list.isFetchingNextPage) void list.fetchNextPage();
          }}
          ListFooterComponent={list.isFetchingNextPage ? <Spinner /> : undefined}
          contentContainerClassName="gap-1.5 px-4 pb-12"
          refreshControl={
            <RefreshControl refreshing={list.isRefetching} onRefresh={() => void list.refetch()} />
          }
          ListEmptyComponent={
            tab === "open" ? (
              <EmptyState
                emoji="🎒"
                title={t("assignments.empty")}
                body={t("assignments.emptyBody")}
                action={<Button onPress={add}>{t("assignments.add")}</Button>}
              />
            ) : (
              <EmptyState emoji="🗂️" title={t("assignments.emptyDone")} />
            )
          }
        />
      )}
    </View>
  );
}
