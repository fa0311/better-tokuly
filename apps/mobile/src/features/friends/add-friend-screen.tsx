import { router } from "expo-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Share, Text, View } from "react-native";
import { Button } from "#/components/button";
import { Page, Section } from "#/components/page";
import { TextField } from "#/components/text-field";
import { useMyInvite, useSendRequest } from "./data";

/** The invite code inside a link (`https://…/invite/<code>`, `tokuly://invite/<code>`) or the code itself. */
export function inviteCodeOf(input: string): string {
  const trimmed = input.trim();
  const match = trimmed.match(/invite\/([^/?#\s]+)/);
  return match?.[1] ?? trimmed;
}

/** Add a friend by handle or by their invite link, and hand out mine. */
export function AddFriendScreen() {
  const { t } = useTranslation("mobile");
  const { t: tf } = useTranslation("friends");
  const [handle, setHandle] = useState("");
  const [invite, setInvite] = useState("");
  const send = useSendRequest();
  const { data } = useMyInvite();
  const url = data?.invite?.url;
  return (
    <Page testID="add-friend-screen">
      <Section title={t("friends.byHandle")}>
        <View className="gap-3 p-4">
          <TextField
            testID="add-friend-handle"
            label={t("friends.handle")}
            placeholder={t("friends.handlePlaceholder")}
            value={handle}
            onChangeText={setHandle}
            autoCapitalize="none"
            autoCorrect={false}
          />
          <Button
            testID="add-friend-send"
            block
            loading={send.isPending}
            disabled={!handle.trim()}
            onPress={() => send.mutate({ handle }, { onSuccess: () => setHandle("") })}
          >
            {t("friends.send")}
          </Button>
        </View>
      </Section>

      <Section title={t("friends.byInvite")}>
        <View className="gap-3 p-4">
          <TextField
            label={t("friends.inviteLink")}
            placeholder={t("friends.invitePlaceholder")}
            value={invite}
            onChangeText={setInvite}
            autoCapitalize="none"
            autoCorrect={false}
          />
          <Button
            variant="secondary"
            block
            disabled={!invite.trim()}
            onPress={() =>
              router.push({ pathname: "/invite/[code]", params: { code: inviteCodeOf(invite) } })
            }
          >
            {t("friends.open")}
          </Button>
        </View>
      </Section>

      <Section title={t("friends.myInvite")}>
        <View className="gap-3 p-4">
          <Text selectable className="text-sm font-bold text-ink" testID="my-invite-url">
            {url ?? "…"}
          </Text>
          {data?.invite && data.invite.introCount > 0 && (
            <Text className="text-xs text-ink-soft">
              {tf("invite.intros", { count: data.invite.introCount })}
            </Text>
          )}
          <Button
            variant="lime"
            block
            disabled={!url}
            onPress={() => url && void Share.share({ message: t("friends.shareMessage", { url }) })}
          >
            {t("friends.shareInvite")}
          </Button>
        </View>
      </Section>
    </Page>
  );
}
