import { useMutation, useQueryClient } from "@tanstack/react-query";
import { school, toDate } from "@tokuly/api";
import {
  dueItems,
  examCountdown,
  momentLabel,
  refreshSchool,
  urgencyKey,
} from "@tokuly/core/school";
import { router } from "expo-router";
import { BellOff, Check } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, View } from "react-native";
import { ActionSheet } from "#/components/action-sheet";
import { Spinner } from "#/components/page";
import { webScreenHref } from "#/features/web/handoff";
import { errorMessage } from "#/lib/errors";
import { INK_ON_BRIGHT, useTheme } from "#/lib/theme";
import {
  asAccount,
  assignmentClient,
  NS,
  useAccountTimeZone,
  useAssignments,
  useExams,
} from "./data";
import { SubjectPill } from "./parts";

const URGENCY_CLASS: Partial<Record<school.AssignmentUrgency, string>> = {
  [school.AssignmentUrgency.OVERDUE]: "bg-danger",
  [school.AssignmentUrgency.TODAY]: "bg-coral",
  [school.AssignmentUrgency.TOMORROW]: "bg-sun",
  [school.AssignmentUrgency.THIS_WEEK]: "bg-sky",
};
const BRIGHT_URGENCY = new Set([
  school.AssignmentUrgency.TOMORROW,
  school.AssignmentUrgency.THIS_WEEK,
]);

const complain = (error: unknown) => Alert.alert(errorMessage(error, { namespace: NS }));

/**
 * One assignment: tick it off, see what it is for and when it is due. A
 * long press offers removing it, or its details on the web.
 */
export function AssignmentRow({ assignment }: { assignment: school.Assignment }) {
  const { t, i18n } = useTranslation(NS);
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const zone = useAccountTimeZone();
  const [menu, setMenu] = useState(false);
  const toggle = useMutation({
    mutationFn: (done: boolean) =>
      asAccount(() => assignmentClient().updateAssignment({ assignmentId: assignment.id, done })),
    onSuccess: (answer) => {
      if (answer) refreshSchool(queryClient, "assignments");
    },
    onError: complain,
  });
  const remove = useMutation({
    mutationFn: () =>
      asAccount(() => assignmentClient().deleteAssignment({ assignmentId: assignment.id })),
    onSuccess: (answer) => {
      if (answer) refreshSchool(queryClient, "assignments");
    },
    onError: complain,
  });
  const done = toggle.isPending ? toggle.variables : assignment.done;
  const due = toDate(assignment.dueAt);
  const urgency = urgencyKey(assignment.urgency);
  const confirmRemove = () =>
    Alert.alert(t("assignments.deleteTitle"), assignment.title, [
      { text: t("common:actions.cancel"), style: "cancel" },
      { text: t("common:actions.delete"), style: "destructive", onPress: () => remove.mutate() },
    ]);
  return (
    <View
      testID={`assignment-${assignment.id}`}
      className={`flex-row items-center gap-3 rounded-2xl bg-surface-2 px-3 py-2.5 ${remove.isPending ? "opacity-50" : ""}`}
    >
      <Pressable
        testID={`assignment-done-${assignment.id}`}
        accessibilityRole="checkbox"
        accessibilityState={{ checked: done }}
        accessibilityLabel={t(done ? "assignments.markOpen" : "assignments.markDone", {
          title: assignment.title,
        })}
        onPress={() => toggle.mutate(!done)}
        className={`size-8 items-center justify-center rounded-full border-2 ${done ? "bg-lime" : "bg-surface"}`}
        style={{ borderColor: colors.ink }}
      >
        {done && <Check size={18} color={INK_ON_BRIGHT} />}
      </Pressable>
      <Pressable
        className="flex-1 gap-0.5"
        accessibilityHint={t("mobile:school.actions")}
        onLongPress={() => setMenu(true)}
      >
        <Text
          numberOfLines={1}
          className={`font-bold ${done ? "text-ink-faint line-through" : "text-ink"}`}
        >
          {assignment.title}
        </Text>
        <View className="flex-row flex-wrap items-center gap-1.5">
          <SubjectPill subject={assignment.subject} />
          {!done && urgency && (
            <Text
              className={`overflow-hidden rounded-full px-2 py-0.5 text-[11px] font-black ${URGENCY_CLASS[assignment.urgency] ?? ""}`}
              style={{ color: BRIGHT_URGENCY.has(assignment.urgency) ? INK_ON_BRIGHT : "#ffffff" }}
            >
              {t(`assignments.urgency.${urgency}`)}
            </Text>
          )}
          {due && (
            <Text className="text-xs font-bold text-ink-soft">
              {t("assignments.dueAt", { when: momentLabel(due, i18n.language, zone) })}
            </Text>
          )}
          {!assignment.remind && !done && (
            <BellOff
              size={14}
              color={colors["ink-soft"]}
              accessibilityLabel={t("assignments.remindOff")}
            />
          )}
        </View>
      </Pressable>
      <ActionSheet
        visible={menu}
        title={assignment.title}
        onClose={() => setMenu(false)}
        actions={[
          {
            key: "edit",
            label: t("mobile:school.editOnWeb"),
            onPress: () => router.push(webScreenHref("/school/assignments") as never),
          },
          {
            key: "delete",
            label: t("common:actions.delete"),
            destructive: true,
            onPress: confirmRemove,
          },
        ]}
      />
    </View>
  );
}

/** "What is due": the exams that are on or coming up and the open assignments, soonest first. */
export function DueList({ limit }: { limit: number }) {
  const { t, i18n } = useTranslation(NS);
  const { colors } = useTheme();
  const zone = useAccountTimeZone();
  const exams = useExams();
  const assignments = useAssignments(school.AssignmentFilter.OPEN);
  if (exams.isPending || assignments.isPending) return <Spinner />;
  const items = dueItems(exams.data ?? [], assignments.data?.assignments ?? [], zone);
  if (items.length === 0)
    return (
      <Text
        testID="school-due-empty"
        className="rounded-card bg-surface-2 px-4 py-5 text-center text-sm font-bold text-ink-soft"
      >
        {t("due.empty")}
      </Text>
    );
  return (
    <View testID="school-due" className="gap-1.5">
      {items.slice(0, limit).map((item) =>
        item.assignment ? (
          <AssignmentRow key={item.assignment.id} assignment={item.assignment} />
        ) : (
          <Pressable
            key={item.exam.id}
            testID={`exam-${item.exam.id}`}
            accessibilityRole="link"
            accessibilityHint={t("mobile:school.onWeb")}
            onPress={() => router.push(webScreenHref(`/school/exams/${item.exam.id}`) as never)}
            className="flex-row items-center gap-3 rounded-2xl bg-surface-2 px-3 py-2.5 active:bg-surface-3"
          >
            <View
              className="size-8 items-center justify-center rounded-full border-2 bg-sun"
              style={{ borderColor: colors.ink }}
            >
              <Text>📝</Text>
            </View>
            <View className="flex-1">
              <Text numberOfLines={1} className="font-bold text-ink">
                {item.exam.name}
              </Text>
              <Text className="text-xs font-bold text-ink-soft">
                {i18n.language.startsWith("ja")
                  ? `${item.exam.startDate.slice(5).replace("-", "/")} 〜`
                  : item.exam.startDate}
              </Text>
            </View>
            <Text
              className={`overflow-hidden rounded-full px-2.5 py-1 text-xs font-black ${item.exam.status === school.ExamStatus.ONGOING ? "bg-live text-white" : "bg-ink text-surface"}`}
            >
              {examCountdown(i18n, item.exam)}
            </Text>
          </Pressable>
        ),
      )}
      {items.length > limit && (
        <Pressable onPress={() => router.push("/school/assignments")}>
          <Text className="px-1 text-right text-sm font-bold text-brand">
            {t("due.more", { count: items.length - limit })}
          </Text>
        </Pressable>
      )}
    </View>
  );
}
