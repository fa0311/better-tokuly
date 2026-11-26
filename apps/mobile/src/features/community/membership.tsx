import { useQueryClient } from "@tanstack/react-query";
import { community, toMillis } from "@tokuly/api";
import { isStaff, refreshMembership, storeCommunity } from "@tokuly/core/community";
import { router } from "expo-router";
import { Bell, Check } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Modal, Pressable, ScrollView, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { ActionSheet } from "#/components/action-sheet";
import { Button } from "#/components/button";
import { TextField } from "#/components/text-field";
import { webScreenHref } from "#/features/web/handoff";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";
import { communityClient, NS } from "./data";

const S = community.MembershipStatus;
const L = community.NotificationLevel;

const LEVELS = [
  { level: L.ALL, key: "all" },
  { level: L.MENTIONS, key: "mentions" },
  { level: L.NONE, key: "none" },
] as const;

/** Shows a refusal of the server in the app's language. */
const complain = (error: unknown) => Alert.alert(errorMessage(error, { namespace: NS }));

/**
 * What the caller can do about their membership, by their standing: join
 * (or ask to), wait for an answer (and take the request back), or — once in
 * — choose what they are notified about and leave.
 */
export function MembershipButton({ community: c }: { community: community.Community }) {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [menu, setMenu] = useState(false);
  const [asking, setAsking] = useState(false);
  const status = c.viewer?.status ?? S.NONE;

  const run = async (work: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await work();
    } catch (error) {
      complain(error);
    } finally {
      setBusy(false);
      refreshMembership(queryClient, c.id);
    }
  };

  const join = (answers: string[] = []) =>
    run(async () => {
      const { status: now, community: joined } = await communityClient().joinCommunity({
        communityId: c.id,
        answers,
      });
      if (joined) storeCommunity(queryClient, joined);
      setAsking(false);
      Alert.alert(now === S.MEMBER ? t("join.welcome") : t("join.requested"));
    });

  const setLevel = (level: community.NotificationLevel) =>
    run(() => communityClient().updateMembership({ communityId: c.id, notificationLevel: level }));

  const leave = () =>
    Alert.alert(
      t("leave.title", { name: c.name }),
      c.visibility === community.CommunityVisibility.PUBLIC
        ? t("leave.body")
        : t("leave.bodyClosed"),
      [
        { text: t("cancel"), style: "cancel" },
        {
          text: t("leave.action"),
          style: "destructive",
          onPress: () => void run(() => communityClient().leaveCommunity({ communityId: c.id })),
        },
      ],
    );

  if (status === S.BANNED) {
    const until = c.viewer?.banExpiresAt;
    return (
      <View testID="community-banned" className="gap-1 rounded-control bg-danger/10 px-4 py-3">
        <Text className="text-sm font-black text-danger">{t("ban.you")}</Text>
        {c.viewer?.banReason ? (
          <Text className="text-xs text-danger">
            {t("ban.reason", { reason: c.viewer.banReason })}
          </Text>
        ) : null}
        <Text className="text-xs text-danger">
          {until
            ? t("ban.until", { time: new Date(toMillis(until)).toLocaleString() })
            : t("ban.forever")}
        </Text>
      </View>
    );
  }

  if (status === S.REQUESTED)
    return (
      <View className="gap-2">
        <Text className="text-xs text-ink-soft">{t("join.requestedHint")}</Text>
        <Button
          testID="community-cancel-request"
          variant="secondary"
          loading={busy}
          onPress={() => void run(() => communityClient().cancelJoinRequest({ communityId: c.id }))}
        >
          {t("join.cancel")}
        </Button>
      </View>
    );

  if (status !== S.MEMBER) {
    const open = c.visibility === community.CommunityVisibility.PUBLIC;
    return (
      <>
        <Button
          testID="community-join"
          loading={busy}
          onPress={() => (open ? void join() : setAsking(true))}
        >
          {open ? t("join.action") : t("join.request")}
        </Button>
        <JoinRequestSheet
          community={c}
          visible={asking}
          busy={busy}
          onSend={(answers) => void join(answers)}
          onClose={() => setAsking(false)}
        />
      </>
    );
  }

  const level = c.viewer?.notificationLevel ?? L.ALL;
  return (
    <>
      <Button
        testID="community-joined"
        variant="secondary"
        loading={busy}
        icon={<Check size={16} color={colors.ink} />}
        onPress={() => setMenu(true)}
      >
        {t("joined")}
      </Button>
      <ActionSheet
        visible={menu}
        title={c.name}
        onClose={() => setMenu(false)}
        actions={[
          ...LEVELS.map(({ level: each, key }) => ({
            key: `notify-${key}`,
            label: `${each === level ? "✓ " : ""}${t("notify.level")}: ${t(`notify.${key}`)}`,
            onPress: () => void setLevel(each),
          })),
          ...(isStaff(c)
            ? [
                {
                  key: "manage",
                  label: t("manage.title"),
                  onPress: () => router.push(webScreenHref(`/g/${c.handle}/manage`) as never),
                },
              ]
            : []),
          { key: "leave", label: t("leave.action"), destructive: true, onPress: leave },
        ]}
      >
        <View className="flex-row items-center gap-2 px-4 pb-2">
          <Bell size={14} color={colors["ink-soft"]} />
          <Text className="flex-1 text-xs text-ink-soft">
            {t(`notify.${LEVELS.find((each) => each.level === level)?.key ?? "all"}Hint`)}
          </Text>
        </View>
      </ActionSheet>
    </>
  );
}

/** Asking to join a community that takes requests: its questions, each answered. */
function JoinRequestSheet({
  community: c,
  visible,
  busy,
  onSend,
  onClose,
}: {
  community: community.Community;
  visible: boolean;
  busy: boolean;
  onSend(answers: string[]): void;
  onClose(): void;
}) {
  const { t } = useTranslation(NS);
  const insets = useSafeAreaInsets();
  const [answers, setAnswers] = useState<string[]>([]);
  const complete = c.joinQuestions.every((_, index) => answers[index]?.trim());
  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={onClose}>
      <Pressable accessibilityRole="none" className="flex-1 bg-black/40" onPress={onClose} />
      <View
        testID="join-request-sheet"
        className="max-h-[80%] rounded-t-3xl bg-surface px-4 pt-4"
        style={{ paddingBottom: insets.bottom + 12 }}
      >
        <ScrollView keyboardShouldPersistTaps="handled" contentContainerClassName="gap-3 pb-2">
          <Text className="text-lg font-black text-ink">
            {t("join.requestTitle", { name: c.name })}
          </Text>
          <Text className="text-sm text-ink-soft">{t("join.requestBody")}</Text>
          {c.joinQuestions.map((question, index) => (
            <TextField
              key={question}
              testID={`join-answer-${index}`}
              label={question}
              multiline
              value={answers[index] ?? ""}
              onChangeText={(text) =>
                setAnswers((current) => {
                  const next = [...current];
                  next[index] = text;
                  return next;
                })
              }
            />
          ))}
          <Button
            testID="join-request-send"
            loading={busy}
            disabled={!complete}
            onPress={() => onSend(c.joinQuestions.map((_, index) => answers[index]?.trim() ?? ""))}
          >
            {t("join.send")}
          </Button>
        </ScrollView>
      </View>
    </Modal>
  );
}
