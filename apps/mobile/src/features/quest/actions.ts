import { useMutation, useQueryClient } from "@tanstack/react-query";
import { quest } from "@tokuly/api";
import { questKeys, showClaim, showReroll } from "@tokuly/core/quest";
import { useRef } from "react";
import { useTranslation } from "react-i18next";
import { Alert } from "react-native";
import { errorMessage } from "#/lib/errors";
import { asAccount, coinsKey } from "./data";

const NS = { namespace: "quest" } as const;

/**
 * Collecting a mission's reward. A mission is sent once while its claim is
 * on its way, however often it is tapped (the server would not grant twice
 * either, but a second request is never made). The coins shown afterwards
 * are the wallet's, read again — never added up here — also when the claim
 * failed, since a lost answer does not mean a refused claim.
 */
export function useClaim() {
  const { t } = useTranslation("quest");
  const queryClient = useQueryClient();
  const sending = useSending();
  const claim = useMutation({
    mutationFn: (mission: quest.Mission) =>
      asAccount((account) =>
        account.api.service(quest.MissionService).claimMission({ missionId: mission.id }),
      ),
    onSuccess: (response, mission) => {
      if (!response) return;
      showClaim(queryClient, response, mission);
      void queryClient.invalidateQueries({ queryKey: coinsKey });
      if (!response.granted) return;
      const coins = (response.reward?.coins ?? 0) + (response.bonus?.coins ?? 0);
      const xp = (response.reward?.xp ?? 0) + (response.bonus?.xp ?? 0);
      Alert.alert(
        t("celebrate.claim.eyebrow"),
        [t("missions.claimed", { coins, xp }), response.bonus ? t("celebrate.claim.bonus") : ""]
          .filter(Boolean)
          .join("\n"),
      );
    },
    onError: (error) => {
      Alert.alert(errorMessage(error, NS));
      // The claim may have gone through with only its answer lost (the app
      // went to the background, the connection dropped): whatever it changed
      // is read again from the server.
      void queryClient.invalidateQueries({ queryKey: questKeys.missions });
      void queryClient.invalidateQueries({ queryKey: questKeys.progress });
      void queryClient.invalidateQueries({ queryKey: coinsKey });
    },
    onSettled: (_answer, _error, mission) => sending.done(mission),
  });
  return {
    claim: (mission: quest.Mission) => {
      if (sending.start(mission)) claim.mutate(mission);
    },
    /** The mission whose claim is on its way, if any. */
    claiming: claim.isPending ? claim.variables?.id : undefined,
  };
}

/**
 * Swapping a mission for another one (once a day), after asking. Like a
 * claim, it is sent once while on its way, however often it is confirmed
 * (two taps on the button open two questions).
 */
export function useReroll() {
  const { t } = useTranslation("quest");
  const queryClient = useQueryClient();
  const sending = useSending();
  const reroll = useMutation({
    mutationFn: (mission: quest.Mission) =>
      asAccount((account) =>
        account.api.service(quest.MissionService).rerollMission({ missionId: mission.id }),
      ),
    onSuccess: (response) => response && showReroll(queryClient, response),
    onError: (error) => {
      Alert.alert(errorMessage(error, NS));
      void queryClient.invalidateQueries({ queryKey: questKeys.missions });
    },
    onSettled: (_answer, _error, mission) => sending.done(mission),
  });
  return {
    ask: (mission: quest.Mission) =>
      Alert.alert(t("missions.rerollConfirm.title"), t("missions.rerollConfirm.body"), [
        { text: t("common:actions.cancel"), style: "cancel" },
        {
          text: t("missions.rerollConfirm.action"),
          onPress: () => {
            if (sending.start(mission)) reroll.mutate(mission);
          },
        },
      ]),
    rerolling: reroll.isPending ? reroll.variables?.id : undefined,
  };
}

/** The missions with a request on its way: `start` is false for one already sent. */
function useSending() {
  const missions = useRef(new Set<string>()).current;
  return {
    start(mission: quest.Mission): boolean {
      if (missions.has(mission.id)) return false;
      missions.add(mission.id);
      return true;
    },
    done: (mission: quest.Mission) => missions.delete(mission.id),
  };
}
