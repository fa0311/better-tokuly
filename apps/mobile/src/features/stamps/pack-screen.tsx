import { useQueryClient } from "@tanstack/react-query";
import type { stamp } from "@tokuly/api";
import { router } from "expo-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Button } from "#/components/button";
import { EmptyState, Spinner } from "#/components/page";
import { webScreenHref } from "#/features/web/handoff";
import { errorMessage } from "#/lib/errors";
import { NS, setInstalled, usePack } from "./data";
import { creatorName, PackCover, PriceTag } from "./parts";
import { StampGrid } from "./picker";

/**
 * A pack and its stamps. A free pack (or one already bought) is added to the
 * picker or taken out here; a paid one is bought with coins on the web
 * screen, as the wallet is the web's for now.
 */
export function PackScreen({ packId }: { packId: string }) {
  const { t } = useTranslation(NS);
  const pack = usePack(packId);
  if (pack.isPending) return <Spinner />;
  if (!pack.data)
    return (
      <EmptyState
        emoji="🫥"
        title={t("pack.notFound")}
        body={t("pack.notFoundBody")}
        action={
          <Button size="sm" onPress={() => router.replace("/stamps")}>
            {t("pack.toStore")}
          </Button>
        }
      />
    );
  const { summary, description, stamps } = pack.data;
  return (
    <SafeAreaView edges={["bottom"]} className="flex-1 bg-bg" testID="stamp-pack">
      <StampGrid
        stamps={stamps.filter((item) => item.stampRef)}
        header={
          summary && <Header pack={summary} description={description} empty={stamps.length === 0} />
        }
      />
    </SafeAreaView>
  );
}

function Header({
  pack,
  description,
  empty,
}: {
  pack: stamp.PackSummary;
  description: string;
  empty: boolean;
}) {
  const { t } = useTranslation(NS);
  return (
    <View className="gap-3 px-1 pb-3">
      <View className="flex-row gap-4">
        <PackCover pack={pack} size={112} />
        <View className="min-w-0 flex-1 justify-center gap-1">
          <Text className="text-xl font-black text-ink">{pack.name}</Text>
          <Text className="text-sm text-ink-soft">{creatorName(t, pack)}</Text>
          <Text className="text-xs text-ink-faint">
            {t("stampCount", { count: pack.stampCount })}
            {pack.animated ? ` · ${t("animated")}` : ""}
          </Text>
          <PriceTag pack={pack} />
        </View>
      </View>
      {description !== "" && <Text className="text-sm leading-5 text-ink">{description}</Text>}
      <PackAction pack={pack} />
      {empty && <Text className="py-6 text-center text-ink-soft">{t("pack.noStamps")}</Text>}
    </View>
  );
}

/** Add, take out, or buy (on the web): what the viewer can do with the pack. */
function PackAction({ pack }: { pack: stamp.PackSummary }) {
  const { t } = useTranslation(NS);
  const { t: tm } = useTranslation("mobile");
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState(false);
  const viewer = pack.viewer;
  if (!viewer) return null;
  if (viewer.canManage)
    return (
      <Button
        testID="pack-studio"
        variant="ghost"
        block
        onPress={() => router.push(webScreenHref(`/stamps/studio/${pack.id}`))}
      >
        {t("pack.editInStudio")}
      </Button>
    );

  const change = async (installed: boolean) => {
    setBusy(true);
    try {
      if (await setInstalled(queryClient, pack.id, installed))
        Alert.alert(installed ? t("toast.installed") : t("toast.uninstalled"));
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: NS }));
    } finally {
      setBusy(false);
    }
  };

  if (viewer.installed)
    return (
      <View className="gap-1.5">
        <Button
          testID="pack-uninstall"
          variant="ghost"
          block
          loading={busy}
          onPress={() => void change(false)}
        >
          {t("pack.uninstall")}
        </Button>
        <Text className="text-center text-xs text-ink-soft">{t("pack.installedHint")}</Text>
      </View>
    );
  if (pack.price === 0 || viewer.owned)
    return (
      <View className="gap-1.5">
        <Button testID="pack-install" block loading={busy} onPress={() => void change(true)}>
          {viewer.owned ? t("pack.reinstall") : t("pack.installFree")}
        </Button>
        {viewer.owned && (
          <Text className="text-center text-xs text-ink-soft">{t("pack.ownedHint")}</Text>
        )}
      </View>
    );
  return (
    <View className="gap-1.5">
      <Button
        testID="pack-buy"
        block
        onPress={() => router.push(webScreenHref(`/stamps/${pack.shortId || pack.id}`))}
      >
        {t("pack.buy", { price: pack.price })}
      </Button>
      <Text className="text-center text-xs text-ink-soft">{tm("stamps.buyOnWeb")}</Text>
    </View>
  );
}
