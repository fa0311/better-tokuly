import { useQueryClient } from "@tanstack/react-query";
import { spaces } from "@tokuly/api";
import type { Voices } from "@tokuly/core/spaces";
import { isModerator, nameOf, onStage, rememberSpace, spacesKeys } from "@tokuly/core/spaces";
import { router } from "expo-router";
import { MicOff } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, View } from "react-native";
import { ActionSheet, type SheetAction } from "#/components/action-sheet";
import { Avatar } from "#/components/avatar";
import { useMe } from "#/features/session/store";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";
import { spaceClient } from "./data";

const Role = spaces.SpaceRole;

export function roleKey(role: spaces.SpaceRole): "host" | "cohost" | "speaker" | "listener" {
  if (role === Role.HOST) return "host";
  if (role === Role.COHOST) return "cohost";
  if (role === Role.SPEAKER) return "speaker";
  return "listener";
}

/**
 * Runs something a person does in a room (a hand, a stage invitation, a
 * mute): failures in the API's words, an optional word of success, and the
 * lists of people read again, since not every change comes back as an event.
 */
export function useRoomAction(spaceId: string) {
  const queryClient = useQueryClient();
  return async (call: () => Promise<unknown>, done?: string) => {
    try {
      await call();
      if (done) Alert.alert(done);
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: "spaces" }));
    }
    void queryClient.invalidateQueries({ queryKey: spacesKeys.everyParticipants(spaceId) });
  };
}

/** Someone on stage: a big face that lights up while they talk. */
export function StageTile({
  participant,
  voices,
  onOpen,
}: {
  participant: spaces.SpaceParticipant;
  voices: Voices;
  onOpen(): void;
}) {
  const { t } = useTranslation("spaces");
  const { colors } = useTheme();
  const user = participant.user;
  const id = user?.id ?? "";
  const speaking = voices.speaking.has(id);
  // Outside the room there is no connection to tell who is muted.
  const muted = voices.connected.has(id) && !voices.open.has(id);
  const role = roleKey(participant.role);
  return (
    <Pressable
      testID="stage-person"
      accessibilityRole="button"
      accessibilityLabel={t(`room.personLabel.${role}`, { name: nameOf(user) })}
      accessibilityState={{ selected: speaking }}
      onPress={onOpen}
      className="w-1/3 items-center gap-1.5 p-2"
    >
      <View
        className="rounded-[22px] p-1"
        style={{ backgroundColor: speaking ? colors.lime : "transparent" }}
      >
        <Avatar user={user} size="lg" outlined />
        {muted && (
          <View
            accessibilityLabel={t("room.mutedLabel")}
            className="absolute -right-1 -bottom-1 size-6 items-center justify-center rounded-full border-2 border-ink bg-surface"
          >
            <MicOff size={13} color={colors["ink-soft"]} />
          </View>
        )}
      </View>
      <Text numberOfLines={1} className="w-full text-center text-sm font-bold text-ink">
        {nameOf(user)}
      </Text>
      <Text
        className={`rounded-full px-2 text-[11px] font-black ${
          participant.role === Role.HOST
            ? "bg-sun text-[#17122b]"
            : participant.role === Role.COHOST
              ? "bg-sky text-[#17122b]"
              : "bg-surface-2 text-ink-soft"
        }`}
      >
        {t(`role.${role}`)}
      </Text>
    </Pressable>
  );
}

/** Someone in the audience: a small face, a raised hand when it is up. */
export function AudienceTile({
  participant,
  onOpen,
}: {
  participant: spaces.SpaceParticipant;
  onOpen(): void;
}) {
  const { t } = useTranslation("spaces");
  return (
    <Pressable
      testID="audience-person"
      accessibilityRole="button"
      accessibilityLabel={t("room.personLabel.listener", { name: nameOf(participant.user) })}
      onPress={onOpen}
      className="w-1/4 items-center gap-1 p-1.5"
    >
      <View>
        <Avatar user={participant.user} outlined />
        {participant.handRaisedAt && (
          <Text
            accessibilityLabel={t("room.handUpLabel")}
            className="absolute -top-2 -right-2 text-lg"
          >
            ✋
          </Text>
        )}
      </View>
      <Text numberOfLines={1} className="w-full text-center text-xs font-bold text-ink-soft">
        {nameOf(participant.user)}
      </Text>
    </Pressable>
  );
}

/**
 * What the viewer can do about one person in the room: see their profile
 * and, as a moderator, bring them on stage, send them back, mute, name a
 * co-host, hand the room over, remove or ban (the last ones asked first).
 */
export function PersonSheet({
  space,
  participant,
  onClose,
}: {
  space: spaces.Space;
  participant: spaces.SpaceParticipant | null;
  onClose(): void;
}) {
  const { t } = useTranslation("spaces");
  const queryClient = useQueryClient();
  const myId = useMe()?.user?.id ?? "";
  const run = useRoomAction(space.id);
  const user = participant?.user;
  if (!participant || !user) return null;
  const spaceId = space.id;
  const userId = user.id;
  const name = nameOf(user);
  const myRole = space.viewer?.role ?? Role.UNSPECIFIED;
  const iAmHost = myRole === Role.HOST;
  // The host cannot be touched; a co-host only by the host.
  const mayModerate =
    space.state === spaces.SpaceState.LIVE &&
    userId !== myId &&
    isModerator(myRole) &&
    participant.role !== Role.HOST &&
    (participant.role !== Role.COHOST || iAmHost);
  const staged = onStage(participant.role);
  const service = spaceClient();

  const confirm = (title: string, body: string, label: string, action: () => void) =>
    Alert.alert(title, body, [
      { text: t("common:actions.cancel"), style: "cancel" },
      { text: label, style: "destructive", onPress: action },
    ]);
  const remove = (ban: boolean) =>
    confirm(
      t(ban ? "confirm.banTitle" : "confirm.removeTitle", { name }),
      t(ban ? "confirm.banBody" : "confirm.removeBody"),
      t(ban ? "person.ban" : "person.remove"),
      () =>
        void run(
          () => service.removeParticipant({ spaceId, userId, ban }),
          t(ban ? "toast.banned" : "toast.removed", { name }),
        ),
    );

  const actions: SheetAction[] = [
    {
      key: "profile",
      label: t("person.profile"),
      onPress: () => router.push(`/u/${user.handle}`),
    },
  ];
  if (mayModerate && !staged) {
    actions.push({
      key: "stage",
      label: t(participant.handRaisedAt ? "person.bringUp" : "person.inviteToStage"),
      onPress: () =>
        void run(
          () => service.inviteToStage({ spaceId, userId }),
          t(participant.handRaisedAt ? "toast.onStage" : "toast.stageInvited", { name }),
        ),
    });
    if (participant.stageInvited || participant.handRaisedAt)
      actions.push({
        key: "lower",
        label: t(participant.stageInvited ? "person.withdrawInvite" : "person.lowerHand"),
        onPress: () => void run(() => service.moveToAudience({ spaceId, userId })),
      });
  }
  if (mayModerate && staged) {
    actions.push(
      {
        key: "mute",
        label: t("person.mute"),
        onPress: () =>
          void run(() => service.muteSpeaker({ spaceId, userId }), t("toast.muted", { name })),
      },
      {
        key: "audience",
        label: t("person.toAudience"),
        onPress: () => void run(() => service.moveToAudience({ spaceId, userId })),
      },
    );
    if (iAmHost)
      actions.push(
        {
          key: "cohost",
          label: t(participant.role === Role.COHOST ? "person.removeCohost" : "person.makeCohost"),
          onPress: () =>
            void run(
              () =>
                service.setCohost({ spaceId, userId, cohost: participant.role !== Role.COHOST }),
              participant.role === Role.COHOST ? undefined : t("toast.cohostNamed", { name }),
            ),
        },
        {
          key: "transfer",
          label: t("person.transferHost"),
          onPress: () =>
            confirm(
              t("confirm.transferTitle", { name }),
              t("confirm.transferBody"),
              t("person.transferHost"),
              () =>
                void run(
                  async () => {
                    const response = await service.transferHost({ spaceId, userId });
                    rememberSpace(queryClient, response.space);
                  },
                  t("toast.transferred", { name }),
                ),
            ),
        },
      );
  }
  if (mayModerate)
    actions.push(
      { key: "remove", label: t("person.remove"), destructive: true, onPress: () => remove(false) },
      { key: "ban", label: t("person.ban"), destructive: true, onPress: () => remove(true) },
    );

  return (
    <ActionSheet
      visible
      title={`${name}・${t(`role.${roleKey(participant.role)}`)}`}
      actions={actions}
      onClose={onClose}
    />
  );
}
