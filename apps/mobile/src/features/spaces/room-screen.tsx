import { useQueryClient } from "@tanstack/react-query";
import { spaces } from "@tokuly/api";
import {
  isModerator,
  nameOf,
  onStage,
  patchSpace,
  rememberSpace,
  sortStage,
} from "@tokuly/core/spaces";
import { router, Stack } from "expo-router";
import { Ellipsis, Hand, LogOut, Mic, MicOff, Pin, Smile } from "lucide-react-native";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Linking, Pressable, ScrollView, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { ActionSheet, type SheetAction } from "#/components/action-sheet";
import { Button } from "#/components/button";
import { EmptyState, Spinner } from "#/components/page";
import { webScreenHref } from "#/features/web/handoff";
import { sticker, useTheme } from "#/lib/theme";
import { BurstLayer, EmojiRow } from "./bursts";
import { spaceClient, useParticipants, useSpace, useSpacesConfig } from "./data";
import { InsideBar, LiveMark } from "./parts";
import { AudienceTile, PersonSheet, StageTile, useRoomAction } from "./people";
import { EndedRoom, ExitNotice, UpcomingRoom } from "./room-states";
import {
  enterSpace,
  expectEnd,
  leaveVenue,
  roleChanged,
  toggleMicrophone,
  useAudioSession,
  useIsInside,
  useVoices,
} from "./session";
import { useSpaceTopic } from "./topic";

const Role = spaces.SpaceRole;
const Filter = spaces.ParticipantFilter;

/** `/spaces/<id>`: a room, whatever its state, followed live while it is open. */
export function SpaceScreen({ spaceId }: { spaceId: string }) {
  const { t } = useTranslation("spaces");
  const query = useSpace(spaceId);
  useSpaceTopic(spaceId);
  const exit = useAudioSession((state) =>
    state.exit?.kind === "space" && state.exit.id === spaceId ? state.exit : null,
  );
  const space = query.data;
  return (
    <View className="flex-1 bg-bg" testID="space-screen">
      <Stack.Screen options={{ title: space?.title ?? "" }} />
      {exit ? (
        <ExitNotice exit={exit} />
      ) : query.isPending ? (
        <Spinner />
      ) : !space ? (
        <EmptyState emoji="🔍" title={t("room.notFound")} body={t("room.notFoundBody")} />
      ) : space.state === spaces.SpaceState.LIVE ? (
        <LiveRoom space={space} />
      ) : space.state === spaces.SpaceState.SCHEDULED ? (
        <UpcomingRoom space={space} />
      ) : (
        <EndedRoom space={space} />
      )}
    </View>
  );
}

/** The top of a live room: what it is about, its pin, how many are inside. */
function RoomHeader({ space }: { space: spaces.Space }) {
  const { t } = useTranslation("spaces");
  const { colors } = useTheme();
  const pin = space.pin;
  return (
    <View className="gap-2 pb-4">
      <View className="flex-row items-center gap-2">
        <LiveMark />
        <Text className="text-xs font-bold text-ink-soft">
          {t("room.listenerCount", { count: space.listenerCount })}
        </Text>
      </View>
      <Text className="text-2xl leading-8 font-black text-ink">{space.title}</Text>
      {space.tags.length > 0 && (
        <Text className="text-sm font-bold text-brand">
          {space.tags.map((tag) => `#${tag}`).join("  ")}
        </Text>
      )}
      {pin && (pin.text || pin.url) && (
        <Pressable
          testID="space-pin"
          accessibilityRole={pin.url ? "link" : "text"}
          disabled={!pin.url}
          onPress={() => void Linking.openURL(pin.url)}
          className="flex-row gap-2 rounded-2xl bg-sun/30 px-3 py-2.5"
        >
          <Pin size={16} color={colors.ink} />
          <View className="flex-1">
            {pin.text ? <Text className="text-sm font-bold text-ink">{pin.text}</Text> : null}
            {pin.url ? (
              <Text numberOfLines={1} className="text-xs font-bold text-brand">
                {pin.url}
              </Text>
            ) : null}
          </View>
        </Pressable>
      )}
    </View>
  );
}

/** A live room: the stage, the hands and the audience, and the controls of the person inside. */
function LiveRoom({ space }: { space: spaces.Space }) {
  const { t } = useTranslation("spaces");
  const { t: tm } = useTranslation("mobile");
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const spaceId = space.id;
  const inside = useIsInside("space", spaceId);
  const active = useAudioSession((state) =>
    state.active?.kind === "space" && state.active.id === spaceId ? state.active : null,
  );
  const voices = useVoices("space", spaceId);
  const config = useSpacesConfig().data;
  const audioOff = config?.audioEnabled === false;
  const run = useRoomAction(spaceId);
  const [entering, setEntering] = useState(false);
  const [person, setPerson] = useState<spaces.SpaceParticipant | null>(null);
  const [sheet, setSheet] = useState<"emoji" | "menu" | null>(null);

  const role = active?.role ?? space.viewer?.role ?? Role.LISTENER;
  const moderator = isModerator(role);
  const staged = onStage(role);
  const hands = useParticipants(spaceId, Filter.HANDS, inside && moderator);
  const audience = useParticipants(spaceId, Filter.AUDIENCE, inside);
  const handRaised = Boolean(space.viewer?.handRaised);

  const enter = async () => {
    setEntering(true);
    await enterSpace(queryClient, space);
    setEntering(false);
  };
  const leave = async () => {
    await leaveVenue(queryClient);
    router.back();
  };
  const end = async () => {
    expectEnd(spaceId);
    await run(async () => {
      const response = await spaceClient().endSpace({ spaceId });
      await leaveVenue(queryClient);
      rememberSpace(queryClient, response.space);
    }, t("toast.ended"));
  };
  const askLeave = () => {
    if (role !== Role.HOST) return void leave();
    Alert.alert(
      t("hostLeave.title"),
      t("hostLeave.body", { minutes: Math.ceil((config?.hostGraceSeconds || 180) / 60) }),
      [
        { text: t("hostLeave.leave"), onPress: () => void leave() },
        { text: t("hostLeave.end"), style: "destructive", onPress: () => void end() },
        { text: t("common:actions.cancel"), style: "cancel" },
      ],
    );
  };
  const setHand = (raised: boolean) =>
    run(async () => {
      await spaceClient().setHandRaised({ spaceId, raised });
      patchSpace(queryClient, spaceId, (loaded) =>
        loaded.viewer ? { ...loaded, viewer: { ...loaded.viewer, handRaised: raised } } : loaded,
      );
    });
  const answerInvite = (accept: boolean) =>
    run(
      async () => {
        const { me } = await spaceClient().respondToStageInvite({ spaceId, accept });
        patchSpace(queryClient, spaceId, (loaded) =>
          loaded.viewer
            ? {
                ...loaded,
                viewer: {
                  ...loaded.viewer,
                  stageInvited: false,
                  handRaised: false,
                  role: me?.role ?? loaded.viewer.role,
                },
              }
            : loaded,
        );
        if (me && onStage(me.role)) roleChanged(queryClient, spaceId, me.role);
      },
      accept ? t("toast.youAreOnStage") : undefined,
    );
  const stepDown = () =>
    run(async () => {
      await spaceClient().moveToAudience({ spaceId, userId: "" });
      patchSpace(queryClient, spaceId, (loaded) =>
        loaded.viewer ? { ...loaded, viewer: { ...loaded.viewer, role: Role.LISTENER } } : loaded,
      );
      roleChanged(queryClient, spaceId, Role.LISTENER);
    }, t("toast.backToAudience"));

  const menu: SheetAction[] = [
    ...(staged && role !== Role.HOST
      ? [{ key: "step-down", label: t("menu.stepDown"), onPress: () => void stepDown() }]
      : []),
    ...(moderator
      ? [
          {
            key: "settings",
            label: tm("spaces.settingsOnWeb"),
            onPress: () => router.push(webScreenHref(`/spaces/${spaceId}`) as never),
          },
        ]
      : []),
    ...(role === Role.HOST
      ? [
          {
            key: "end",
            label: t("menu.end"),
            destructive: true,
            onPress: () =>
              Alert.alert(t("confirm.endTitle"), t("confirm.endBody"), [
                { text: t("common:actions.cancel"), style: "cancel" },
                { text: t("menu.end"), style: "destructive", onPress: () => void end() },
              ]),
          },
        ]
      : []),
  ];

  const stage = sortStage(space.stage);
  const listeners = audience.people;
  return (
    <SafeAreaView edges={["bottom"]} className="flex-1">
      <ScrollView contentContainerClassName="px-4 pt-3 pb-8">
        <RoomHeader space={space} />
        {space.viewer?.stageInvited && inside && (
          <View
            testID="stage-invitation"
            className="mb-4 gap-3 rounded-card bg-lime/50 p-4"
            style={sticker(colors, "sm")}
          >
            <Text className="text-base font-black text-ink">{t("room.stageInvite")}</Text>
            <View className="flex-row gap-2">
              <Button testID="stage-accept" size="sm" onPress={() => void answerInvite(true)}>
                {t("room.stageAccept")}
              </Button>
              <Button size="sm" variant="ghost" onPress={() => void answerInvite(false)}>
                {t("room.stageDecline")}
              </Button>
            </View>
          </View>
        )}
        <Text className="pb-1 text-xs font-black tracking-wider text-ink-soft uppercase">
          {t("room.stage")}
        </Text>
        {stage.length === 0 ? (
          <Text className="pb-4 text-sm text-ink-soft">{tm("spaces.stageEmpty")}</Text>
        ) : (
          <View testID="space-stage" className="-mx-2 flex-row flex-wrap pb-4">
            {stage.map((participant) => (
              <StageTile
                key={participant.user?.id}
                participant={participant}
                voices={voices}
                onOpen={() => setPerson(participant)}
              />
            ))}
          </View>
        )}
        {moderator && inside && hands.people.length > 0 && (
          <View testID="space-hands" className="gap-2 pb-4">
            <Text className="text-xs font-black tracking-wider text-ink-soft uppercase">
              {t("room.hands")}
            </Text>
            {hands.people.map((participant) => (
              <View
                key={participant.user?.id}
                className="flex-row items-center gap-3 rounded-2xl bg-surface px-3 py-2"
              >
                <Text className="text-lg">✋</Text>
                <Text numberOfLines={1} className="flex-1 text-sm font-bold text-ink">
                  {nameOf(participant.user)}
                </Text>
                <Button
                  size="sm"
                  testID="hand-bring-up"
                  onPress={() =>
                    void run(
                      () =>
                        spaceClient().inviteToStage({
                          spaceId,
                          userId: participant.user?.id ?? "",
                        }),
                      t("toast.onStage", { name: nameOf(participant.user) }),
                    )
                  }
                >
                  {t("room.bringUp")}
                </Button>
              </View>
            ))}
          </View>
        )}
        <Text className="pb-1 text-xs font-black tracking-wider text-ink-soft uppercase">
          {t("room.audience")}
        </Text>
        {inside ? (
          <>
            <View testID="space-audience" className="-mx-1.5 flex-row flex-wrap">
              {listeners.map((participant) => (
                <AudienceTile
                  key={participant.user?.id}
                  participant={participant}
                  onOpen={() => setPerson(participant)}
                />
              ))}
            </View>
            {audience.hiddenCount > 0 ? (
              <Text className="pt-2 text-xs font-bold text-ink-soft">
                {t(listeners.length ? "room.hiddenMore" : "room.hiddenOnly", {
                  count: audience.hiddenCount,
                })}
              </Text>
            ) : listeners.length === 0 && !audience.paging.isPending ? (
              <Text className="text-sm text-ink-soft">{t("room.audienceEmpty")}</Text>
            ) : null}
          </>
        ) : (
          <Text className="text-sm text-ink-soft">
            {t("room.audienceOutside", { count: space.listenerCount })}
          </Text>
        )}
      </ScrollView>
      <BurstLayer spaceId={spaceId} />
      <InsideBar hereKind="space" hereId={spaceId} />
      <View className="gap-2 border-t border-line bg-surface px-4 pt-3 pb-2">
        {!inside ? (
          <>
            <Button
              testID="space-join"
              size="lg"
              block
              loading={entering}
              disabled={audioOff}
              onPress={() => void enter()}
            >
              {t(space.viewer?.inside || moderator ? "room.rejoin" : "room.join")}
            </Button>
            <Text className="text-center text-xs text-ink-soft">
              {audioOff ? t("audioOff") : t("room.joinHint")}
            </Text>
          </>
        ) : (
          <View className="flex-row items-center gap-2">
            <Button
              testID="space-leave"
              variant="ghost"
              icon={<LogOut size={16} color={colors.ink} />}
              onPress={askLeave}
            >
              {t("room.leave")}
            </Button>
            <View className="flex-1 items-center">
              {active?.phase !== "connected" && (
                <Text className="text-xs font-bold text-ink-soft">
                  {t(active?.phase === "reconnecting" ? "room.reconnecting" : "room.connecting")}
                </Text>
              )}
            </View>
            {staged ? (
              <RoundControl
                testID="space-mic"
                label={t(active?.micOn ? "room.mute" : "room.unmute")}
                on={Boolean(active?.micOn)}
                onPress={() => void toggleMicrophone(queryClient)}
                icon={
                  active?.micOn ? (
                    <Mic size={22} color={colors["on-brand"]} />
                  ) : (
                    <MicOff size={22} color={colors.ink} />
                  )
                }
              />
            ) : (
              <RoundControl
                testID="space-hand"
                label={t(handRaised ? "room.handDown" : "room.handUp")}
                on={handRaised}
                onPress={() => void setHand(!handRaised)}
                icon={<Hand size={22} color={handRaised ? colors["on-brand"] : colors.ink} />}
              />
            )}
            <RoundControl
              testID="space-react"
              label={t("room.reactions")}
              on={false}
              onPress={() => setSheet("emoji")}
              icon={<Smile size={22} color={colors.ink} />}
            />
            {menu.length > 0 && (
              <RoundControl
                testID="space-menu"
                label={t("room.more")}
                on={false}
                onPress={() => setSheet("menu")}
                icon={<Ellipsis size={22} color={colors.ink} />}
              />
            )}
          </View>
        )}
      </View>
      <ActionSheet
        visible={sheet === "emoji"}
        title={t("room.reactions")}
        actions={[]}
        onClose={() => setSheet(null)}
      >
        <EmojiRow spaceId={spaceId} onSent={() => setSheet(null)} />
      </ActionSheet>
      <ActionSheet visible={sheet === "menu"} actions={menu} onClose={() => setSheet(null)} />
      <PersonSheet space={space} participant={person} onClose={() => setPerson(null)} />
    </SafeAreaView>
  );
}

/** A round control of the bottom bar: filled while it is on. */
function RoundControl({
  icon,
  label,
  on,
  onPress,
  testID,
}: {
  icon: ReactNode;
  label: string;
  on: boolean;
  onPress(): void;
  testID: string;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ selected: on }}
      onPress={onPress}
      hitSlop={4}
      className={`size-12 items-center justify-center rounded-full ${on ? "bg-brand" : "bg-surface-2"}`}
      style={on ? sticker(colors, "sm") : undefined}
    >
      {icon}
    </Pressable>
  );
}
