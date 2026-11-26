import { contextOfSearch, type FeedContext, validateShortSearch } from "@tokuly/core/shorts";
import { router } from "expo-router";
import { Clapperboard, Plus } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { ShortsFeed } from "./feed";

const TABS = ["forYou", "following", "friends"] as const;

/** The shorts tab: the three swipe feeds, with posting and the creator's own list one tap away. */
export function ShortsTabScreen() {
  const { t } = useTranslation("shorts");
  const [tab, setTab] = useState<(typeof TABS)[number]>("forYou");
  return (
    <View className="flex-1 bg-black">
      <ShortsFeed key={tab} context={{ kind: tab }} />
      <SafeAreaView edges={["top"]} pointerEvents="box-none" className="absolute inset-x-0 top-0">
        <View pointerEvents="box-none" className="flex-row items-center justify-between px-3 pt-1">
          <Pressable
            testID="shorts-mine"
            accessibilityRole="button"
            accessibilityLabel={t("bar.mine")}
            onPress={() => router.push("/shorts/mine")}
            className="rounded-full bg-black/30 p-2"
          >
            <Clapperboard color="white" size={22} />
          </Pressable>
          <View className="flex-row gap-3">
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
          <Pressable
            testID="shorts-new"
            accessibilityRole="button"
            accessibilityLabel={t("bar.post")}
            onPress={() => router.push("/shorts/new")}
            className="rounded-full bg-black/30 p-2"
          >
            <Plus color="white" size={22} />
          </Pressable>
        </View>
      </SafeAreaView>
    </View>
  );
}

/** A short opened from a grid (a hashtag, a sound, a profile) or by its link, in the list it came from. */
export function WatchScreen({ search }: { search: Record<string, string | undefined> }) {
  const context: FeedContext = contextOfSearch(validateShortSearch(search));
  return (
    <View className="flex-1 bg-black">
      <ShortsFeed context={context} startId={search.start} />
    </View>
  );
}
