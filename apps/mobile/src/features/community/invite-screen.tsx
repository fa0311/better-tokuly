import { useQueryClient } from "@tanstack/react-query";
import { community } from "@tokuly/api";
import { categoryOf, isMember, refreshMembership, storeCommunity } from "@tokuly/core/community";
import { router } from "expo-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { EmptyState, Spinner } from "#/components/page";
import { errorMessage } from "#/lib/errors";
import { communityClient, NS, useInvitePreview } from "./data";
import { CommunityIcon, useMemberCount } from "./parts";

/**
 * `/g/join/<code>`: what an invite opens — the community and who invited —
 * and one button. An invite skips the join questions and the approval.
 */
export function InviteScreen({ code }: { code: string }) {
  const { t } = useTranslation(NS);
  const queryClient = useQueryClient();
  const members = useMemberCount();
  const preview = useInvitePreview(code);
  const [joining, setJoining] = useState(false);
  const c = preview.data?.community;

  if (preview.isPending) return <Spinner />;
  if (!c)
    return (
      <View className="flex-1 bg-bg" testID="invite-invalid">
        <EmptyState emoji="🎟️" title={t("invite.invalid")} body={t("invite.invalidBody")} />
      </View>
    );

  const open = () => router.replace(`/g/${c.handle}`);
  const join = async () => {
    setJoining(true);
    try {
      const { community: joined, status } = await communityClient().joinCommunity({
        inviteCode: code,
      });
      if (joined) storeCommunity(queryClient, joined);
      refreshMembership(queryClient, joined?.id);
      if (status === community.MembershipStatus.MEMBER) Alert.alert(t("join.welcome"));
      open();
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: NS }));
    } finally {
      setJoining(false);
    }
  };

  const category = categoryOf(c.category);
  const inviter = preview.data?.inviter;
  return (
    <View className="flex-1 items-center gap-4 bg-bg px-6 pt-12" testID="invite-screen">
      {inviter && (
        <View className="flex-row items-center gap-2">
          <Avatar user={inviter} size="sm" />
          <Text className="text-sm font-bold text-ink-soft">
            {t("invite.from", { name: inviter.displayName || inviter.handle })}
          </Text>
        </View>
      )}
      <CommunityIcon community={c} size="lg" />
      <Text className="text-center text-2xl font-black text-ink">{c.name}</Text>
      <Text className="text-sm text-ink-soft">
        {`${category.emoji} ${t(`category.${category.key}`)} · ${members(c.memberCount)}`}
      </Text>
      {c.description !== "" && (
        <Text numberOfLines={5} className="text-center text-sm leading-5 text-ink">
          {c.description}
        </Text>
      )}
      <View className="mt-4 w-full">
        {isMember(c) ? (
          <Button testID="invite-open" size="lg" block onPress={open}>
            {t("invite.open")}
          </Button>
        ) : (
          <Button
            testID="invite-join"
            size="lg"
            block
            loading={joining}
            onPress={() => void join()}
          >
            {t("invite.join")}
          </Button>
        )}
      </View>
    </View>
  );
}
