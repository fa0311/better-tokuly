import { useQuery } from "@tanstack/react-query";
import { identity, social } from "@tokuly/api";
import { friendKeys } from "@tokuly/core/friends";
import { useTranslation } from "react-i18next";
import { Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { EmptyState, Page, Spinner } from "#/components/page";
import { friendClient, useSendRequest } from "./data";

/** An invite link opened in the app: whose it is, and one tap to send them a request. */
export function InviteScreen({ code }: { code: string }) {
  const { t } = useTranslation("mobile");
  const send = useSendRequest();
  const { data, isPending, isError } = useQuery({
    queryKey: [...friendKeys.all, "invite", "resolve", code],
    queryFn: () => friendClient().resolveInvite({ code }),
  });
  if (isPending) return <Spinner />;
  const inviter = data?.inviter;
  if (isError || !inviter) return <EmptyState emoji="🤔" title={t("friends.invalidInvite")} />;
  const friends = data.relationship === identity.Relationship.FRIENDS;
  return (
    <Page testID="invite-screen">
      <View className="items-center gap-3 py-6">
        <Avatar user={inviter} size="xl" outlined />
        <Text className="text-center text-xl font-black text-ink">
          {t("friends.invitedBy", { name: inviter.displayName })}
        </Text>
        <Text className="text-center text-sm text-ink-soft">{t("friends.inviteLead")}</Text>
      </View>
      {friends ? (
        <Text className="text-center font-black text-success">{t("friends.alreadyFriends")}</Text>
      ) : (
        <Button
          testID="invite-send"
          size="lg"
          block
          loading={send.isPending}
          disabled={send.isSuccess}
          onPress={() =>
            send.mutate({ userId: inviter.id, source: social.FriendSource.INVITE_LINK })
          }
        >
          {t("friends.send")}
        </Button>
      )}
    </Page>
  );
}
