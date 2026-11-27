import { useQueryClient } from "@tanstack/react-query";
import { stamp } from "@tokuly/api";
import { router } from "expo-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, FlatList, Pressable, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Button } from "#/components/button";
import { EmptyState, Row, Section, Spinner } from "#/components/page";
import { webScreenHref } from "#/features/web/handoff";
import { errorMessage } from "#/lib/errors";
import { NS, setInstalled, useInstalled } from "./data";
import { PackRow } from "./parts";

/**
 * マイスタンプ: the packs in the person's picker, each of which can be taken
 * out. Making stamps, favourites, purchases and the order of the picker are
 * on the web screens for now.
 */
export function LibraryScreen() {
  const { t } = useTranslation(NS);
  const { t: tm } = useTranslation("mobile");
  const installed = useInstalled();
  const packs = installed.data?.pages.flatMap((page) => page.packs) ?? [];
  return (
    <SafeAreaView edges={["bottom"]} className="flex-1 bg-bg" testID="stamp-library-screen">
      <FlatList
        data={packs}
        keyExtractor={(pack) => pack.id}
        onEndReached={() =>
          installed.hasNextPage && !installed.isFetchingNextPage && installed.fetchNextPage()
        }
        ListHeaderComponent={
          <View className="p-4">
            <Section>
              <Row
                testID="stamp-library-web"
                label={tm("stamps.moreOnWeb")}
                hint={tm("stamps.moreOnWebHint")}
                onPress={() => router.push(webScreenHref("/stamps/library"))}
              />
            </Section>
          </View>
        }
        ListEmptyComponent={
          installed.isPending ? (
            <Spinner />
          ) : (
            <EmptyState
              emoji="🧸"
              title={t("library.packsEmpty")}
              body={t("library.packsEmptyBody")}
              action={
                <Button size="sm" onPress={() => router.push("/stamps")}>
                  {t("library.toStore")}
                </Button>
              }
            />
          )
        }
        renderItem={({ item }) => <PackRow pack={item} trailing={<Uninstall pack={item} />} />}
      />
    </SafeAreaView>
  );
}

function Uninstall({ pack }: { pack: stamp.PackSummary }) {
  const { t } = useTranslation(NS);
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState(false);
  const run = async () => {
    setBusy(true);
    try {
      await setInstalled(queryClient, pack.id, false);
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: NS }));
    } finally {
      setBusy(false);
    }
  };
  // A pack no longer in the store cannot be added again once it is out.
  const gone = pack.status !== stamp.PackStatus.PUBLISHED;
  return (
    <Pressable
      testID={`pack-uninstall-${pack.id}`}
      accessibilityRole="button"
      disabled={busy}
      onPress={() =>
        gone
          ? Alert.alert(
              t("library.uninstallGoneTitle"),
              t("library.uninstallGoneBody", { name: pack.name }),
              [
                { text: t("common:actions.cancel"), style: "cancel" },
                { text: t("pack.uninstall"), style: "destructive", onPress: () => void run() },
              ],
            )
          : void run()
      }
      className={`rounded-full bg-surface-2 px-3 py-1.5 ${busy ? "opacity-50" : ""}`}
    >
      <Text className="text-sm font-black text-ink">{t("pack.uninstall")}</Text>
    </Pressable>
  );
}
