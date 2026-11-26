import { useQueryClient } from "@tanstack/react-query";
import { spaces } from "@tokuly/api";
import { forgetHangout, nameOf, rememberHangout } from "@tokuly/core/spaces";
import { router, Stack } from "expo-router";
import { LogOut, Mic, MicOff, Settings } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, ScrollView, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { EmptyState, Spinner } from "#/components/page";
import { webScreenHref } from "#/features/web/handoff";
import { errorMessage } from "#/lib/errors";
import { sticker, useTheme } from "#/lib/theme";
import { hangoutClient, useHangout, useSpacesConfig } from "./data";
import { InsideBar } from "./parts";
import { ExitNotice } from "./room-states";
import {
  enterHangout,
  leaveVenue,
  toggleMicrophone,
  useAudioSession,
  useIsInside,
  useVoices,
} from "./session";

const Member = spaces.HangoutMemberState;

/** `/spaces/hangouts/<id>`: a hangout, its voice room and its members. */
export function HangoutScreen({ hangoutId }: { hangoutId: string }) {
  const { t } = useTranslation("spaces");
  const query = useHangout(hangoutId);
  const exit = useAudioSession((state) =>
    state.exit?.kind === "hangout" && state.exit.id === hangoutId ? state.exit : null,
  );
  const hangout = query.data;
  return (
    <View className="flex-1 bg-bg" testID="hangout-screen">
      <Stack.Screen options={{ title: hangout ? `${hangout.emoji} ${hangout.name}`.trim() : "" }} />
      {exit ? (
        <ExitNotice exit={exit} />
      ) : query.isPending ? (
        <Spinner />
      ) : !hangout ? (
        <EmptyState emoji="🔍" title={t("hangout.notFound")} body={t("hangout.notFoundBody")} />
      ) : hangout.viewer?.state === Member.INVITED ? (
        <Invitation hangout={hangout} />
      ) : (
        <VoiceRoom hangout={hangout} />
      )}
    </View>
  );
}

/** An invitation to a hangout: who asked, and the answer. */
function Invitation({ hangout }: { hangout: spaces.Hangout }) {
  const { t } = useTranslation("spaces");
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState(false);
  const respond = async (accept: boolean) => {
    setBusy(true);
    try {
      const response = await hangoutClient().respondToHangoutInvite({
        hangoutId: hangout.id,
        accept,
      });
      if (accept) {
        rememberHangout(queryClient, response.hangout);
        Alert.alert(t("toast.hangoutJoined"));
      } else {
        forgetHangout(queryClient, hangout.id);
        router.back();
      }
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: "spaces" }));
    }
    setBusy(false);
  };
  return (
    <EmptyState
      emoji={hangout.emoji || "🛋️"}
      title={t("hangout.invitedTitle", { name: nameOf(hangout.owner) })}
      body={t("hangout.invitedBody")}
      action={
        <View className="flex-row gap-2">
          <Button testID="hangout-accept" loading={busy} onPress={() => void respond(true)}>
            {t("hangouts.accept")}
          </Button>
          <Button variant="ghost" disabled={busy} onPress={() => void respond(false)}>
            {t("hangouts.decline")}
          </Button>
        </View>
      }
    />
  );
}

/** A member's hangout: who is inside (lit while they talk), everyone else, and the way in. */
function VoiceRoom({ hangout }: { hangout: spaces.Hangout }) {
  const { t } = useTranslation("spaces");
  const { t: tm } = useTranslation("mobile");
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const inside = useIsInside("hangout", hangout.id);
  const active = useAudioSession((state) =>
    state.active?.kind === "hangout" && state.active.id === hangout.id ? state.active : null,
  );
  const voices = useVoices("hangout", hangout.id);
  const audioOff = useSpacesConfig().data?.audioEnabled === false;
  const [entering, setEntering] = useState(false);
  const members = hangout.members.filter((member) => member.state === Member.MEMBER);
  const here = members.filter((member) => member.inside);
  const away = members.filter((member) => !member.inside);
  const open = hangout.insideCount > 0;

  const enter = async () => {
    setEntering(true);
    await enterHangout(queryClient, hangout);
    setEntering(false);
  };

  return (
    <SafeAreaView edges={["bottom"]} className="flex-1">
      <ScrollView contentContainerClassName="gap-5 px-4 pt-4 pb-8">
        <View className="items-center gap-2">
          <View
            className={`size-24 items-center justify-center rounded-[28px] ${open ? "bg-lime" : "bg-surface-2"}`}
            style={sticker(colors)}
          >
            <Text className="text-5xl">{hangout.emoji || "🛋️"}</Text>
          </View>
          <Text className="text-2xl font-black text-ink">{hangout.name}</Text>
          <Text className={`text-sm font-black ${open ? "text-success" : "text-ink-soft"}`}>
            {open ? t("hangout.open") : t("hangout.closed")}
          </Text>
        </View>
        <View className="gap-2">
          <Text className="text-xs font-black tracking-wider text-ink-soft uppercase">
            {t("hangout.insideNow")}
          </Text>
          {here.length === 0 ? (
            <Text className="text-sm text-ink-soft">{t("hangout.nobody")}</Text>
          ) : (
            <View testID="hangout-inside" className="-mx-2 flex-row flex-wrap">
              {here.map((member) => {
                const id = member.user?.id ?? "";
                const speaking = voices.speaking.has(id);
                const muted = voices.connected.has(id) && !voices.open.has(id);
                return (
                  <View key={id} className="w-1/3 items-center gap-1.5 p-2">
                    <View
                      className="rounded-[22px] p-1"
                      style={{ backgroundColor: speaking ? colors.lime : "transparent" }}
                    >
                      <Avatar user={member.user} size="lg" outlined />
                      {muted && (
                        <View className="absolute -right-1 -bottom-1 size-6 items-center justify-center rounded-full border-2 border-ink bg-surface">
                          <MicOff size={13} color={colors["ink-soft"]} />
                        </View>
                      )}
                    </View>
                    <Text
                      numberOfLines={1}
                      className="w-full text-center text-sm font-bold text-ink"
                    >
                      {nameOf(member.user)}
                    </Text>
                  </View>
                );
              })}
            </View>
          )}
        </View>
        {away.length > 0 && (
          <View className="gap-2">
            <Text className="text-xs font-black tracking-wider text-ink-soft uppercase">
              {t("hangout.members")}
            </Text>
            <View className="overflow-hidden rounded-card bg-surface">
              {away.map((member) => (
                <Pressable
                  key={member.user?.id}
                  accessibilityRole="button"
                  onPress={() => member.user && router.push(`/u/${member.user.handle}`)}
                  className="flex-row items-center gap-3 border-b border-line px-4 py-3 active:bg-surface-2"
                >
                  <Avatar user={member.user} size="sm" />
                  <Text numberOfLines={1} className="flex-1 text-base font-bold text-ink">
                    {nameOf(member.user)}
                  </Text>
                  {member.owner && (
                    <Text className="rounded-full bg-sun px-2 text-[11px] font-black text-[#17122b]">
                      {t("hangout.owner")}
                    </Text>
                  )}
                </Pressable>
              ))}
            </View>
          </View>
        )}
        <Button
          variant="ghost"
          icon={<Settings size={16} color={colors.ink} />}
          onPress={() => router.push(webScreenHref(`/spaces/hangouts/${hangout.id}`) as never)}
        >
          {tm("spaces.hangoutOnWeb")}
        </Button>
      </ScrollView>
      <InsideBar hereKind="hangout" hereId={hangout.id} />
      <View className="flex-row items-center gap-2 border-t border-line bg-surface px-4 pt-3 pb-2">
        {!inside ? (
          <View className="flex-1 gap-1">
            <Button
              testID="hangout-join"
              size="lg"
              block
              loading={entering}
              disabled={audioOff}
              onPress={() => void enter()}
            >
              {t(open ? "hangout.join" : "hangout.joinFirst")}
            </Button>
            {audioOff && <Text className="text-center text-xs text-ink-soft">{t("audioOff")}</Text>}
          </View>
        ) : (
          <>
            <Button
              testID="hangout-leave"
              variant="ghost"
              icon={<LogOut size={16} color={colors.ink} />}
              onPress={() => void leaveVenue(queryClient)}
            >
              {t("hangout.leave")}
            </Button>
            <View className="flex-1 items-center">
              {active?.phase !== "connected" && (
                <Text className="text-xs font-bold text-ink-soft">
                  {t(active?.phase === "reconnecting" ? "room.reconnecting" : "room.connecting")}
                </Text>
              )}
            </View>
            <Pressable
              testID="hangout-mic"
              accessibilityRole="button"
              accessibilityLabel={t(active?.micOn ? "room.mute" : "room.unmute")}
              accessibilityState={{ selected: Boolean(active?.micOn) }}
              onPress={() => void toggleMicrophone(queryClient)}
              className={`size-14 items-center justify-center rounded-full ${active?.micOn ? "bg-brand" : "bg-surface-2"}`}
              style={sticker(colors, "sm")}
            >
              {active?.micOn ? (
                <Mic size={24} color={colors["on-brand"]} />
              ) : (
                <MicOff size={24} color={colors.ink} />
              )}
            </Pressable>
          </>
        )}
      </View>
    </SafeAreaView>
  );
}
