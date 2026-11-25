import type { social } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { View } from "react-native";
import { Button } from "#/components/button";
import { useCancelRequest, useRespond } from "./data";

/** Accept or decline a request to me; take back one I sent. */
export function RequestActions({
  request,
  incoming,
}: {
  request: social.FriendRequest;
  incoming: boolean;
}) {
  const { t } = useTranslation("friends");
  const respond = useRespond();
  const cancel = useCancelRequest();
  if (!incoming)
    return (
      <Button
        size="sm"
        variant="secondary"
        loading={cancel.isPending}
        onPress={() => cancel.mutate(request.id)}
      >
        {t("requests.cancel")}
      </Button>
    );
  const busy = respond.isPending;
  return (
    <View className="flex-row gap-1.5">
      <Button
        size="sm"
        testID={`accept-${request.from?.handle}`}
        loading={busy && respond.variables?.accept === true}
        disabled={busy}
        onPress={() => respond.mutate({ requestId: request.id, accept: true })}
      >
        {t("requests.accept")}
      </Button>
      <Button
        size="sm"
        variant="secondary"
        testID={`decline-${request.from?.handle}`}
        loading={busy && respond.variables?.accept === false}
        disabled={busy}
        onPress={() => respond.mutate({ requestId: request.id, accept: false })}
      >
        {t("requests.decline")}
      </Button>
    </View>
  );
}
