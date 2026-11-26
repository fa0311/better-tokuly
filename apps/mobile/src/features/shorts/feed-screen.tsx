import { contextOfSearch, type FeedContext, validateShortSearch } from "@tokuly/core/shorts";
import { router } from "expo-router";
import { Clapperboard, Plus } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { IconButton } from "#/components/icon-button";
import { DarkStage } from "#/components/stage";
import { ShortsFeed } from "./feed";

const TABS = ["forYou", "following", "friends"] as const;

/** The shorts tab: the three swipe feeds, with posting and the creator's own list one tap away. */
export function ShortsTabScreen() {
  const { t } = useTranslation("shorts");
  const [tab, setTab] = useState<(typeof TABS)[number]>("forYou");
  return (
    <DarkStage>
      <ShortsFeed key={tab} context={{ kind: tab }} />
      <SafeAreaView edges={["top"]} pointerEvents="box-none" className="absolute inset-x-0 top-0">
        <View pointerEvents="box-none" className="flex-row items-center justify-between px-3 pt-1">
          <IconButton
            testID="shorts-mine"
            look="glass"
            label={t("bar.mine")}
            icon={<Clapperboard color="white" size={22} />}
            onPress={() => router.push("/shorts/mine")}
          />
          <View className="flex-row gap-4">
            {TABS.map((key) => (
              <Pressable
                key={key}
                testID={`shorts-tab-${key}`}
                accessibilityRole="tab"
                accessibilityState={{ selected: tab === key }}
                onPress={() => setTab(key)}
              >
                <Text
                  className={`text-base font-black ${tab === key ? "text-white" : "text-white/60"}`}
                >
                  {t(`tabs.${key}`)}
                </Text>
              </Pressable>
            ))}
          </View>
          <IconButton
            testID="shorts-new"
            look="glass"
            label={t("bar.post")}
            icon={<Plus color="white" size={22} />}
            onPress={() => router.push("/shorts/new")}
          />
        </View>
      </SafeAreaView>
    </DarkStage>
  );
}

/** A short opened from a grid (a hashtag, a sound, a profile) or by its link, in the list it came from. */
export function WatchScreen({ search }: { search: Record<string, string | undefined> }) {
  const context: FeedContext = contextOfSearch(validateShortSearch(search));
  return (
    <DarkStage>
      <ShortsFeed context={context} startId={search.start} />
    </DarkStage>
  );
}
