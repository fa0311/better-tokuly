import { useMutation, useQueryClient } from "@tanstack/react-query";
import { historyKeys } from "@tokuly/core/history";
import { placeKeys } from "@tokuly/core/places";
import { router } from "expo-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Text } from "react-native";
import { Button } from "#/components/button";
import { Page, Spinner } from "#/components/page";
import { TextField } from "#/components/text-field";
import { errorMessage } from "#/lib/errors";
import { asAccount, NS, placeClient, usePlace } from "./data";
import { PlaceBubble } from "./parts";

const MAX_MESSAGE = 200;

/**
 * `/places/<id>/check-in`: "I'm here" at a place, with a few words. It
 * reaches the friends who see the account's precise location (the server
 * applies ghost mode); tagging friends stays on the web.
 */
export function CheckInScreen({ placeId }: { placeId: string }) {
  const { t } = useTranslation(NS);
  const queryClient = useQueryClient();
  const place = usePlace(placeId).data?.place;
  const [message, setMessage] = useState("");
  const checkIn = useMutation({
    mutationFn: () =>
      asAccount(() =>
        placeClient().checkIn({
          where: { case: "placeId", value: placeId },
          message: message.trim(),
        }),
      ),
    onSuccess: (answer) => {
      if (!answer) return;
      void queryClient.invalidateQueries({ queryKey: placeKeys.all });
      void queryClient.invalidateQueries({ queryKey: historyKeys.all });
      Alert.alert(t("checkIn.done"));
      router.back();
    },
    onError: (error) => Alert.alert(errorMessage(error, { namespace: NS })),
  });
  if (!place) return <Spinner />;
  return (
    <Page testID="check-in">
      <PlaceBubble place={place} size="lg" />
      <Text className="text-xl font-black text-ink">{place.name}</Text>
      <Text className="text-sm text-ink-soft">{t("checkIn.lead")}</Text>
      <TextField
        testID="check-in-message"
        label={t("checkIn.message")}
        placeholder={t("checkIn.messagePlaceholder")}
        value={message}
        maxLength={MAX_MESSAGE}
        onChangeText={setMessage}
      />
      <Text className="text-xs text-ink-faint">{t("checkIn.audience")}</Text>
      <Button
        testID="check-in-submit"
        variant="lime"
        block
        size="lg"
        loading={checkIn.isPending}
        onPress={() => checkIn.mutate()}
      >
        {t("checkIn.submit")}
      </Button>
    </Page>
  );
}
