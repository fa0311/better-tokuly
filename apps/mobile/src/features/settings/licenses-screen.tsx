import { FlashList } from "@shopify/flash-list";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Linking, Pressable, Text, TextInput, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import notices from "#/generated/licenses.json";
import { useTheme } from "#/lib/theme";

/** One bundled dependency, as `scripts/licenses.js` wrote it at build time. */
interface Notice {
  name: string;
  version: string;
  license: string;
  url: string;
  /** Index into `texts`; -1 when the package ships no licence file. */
  text: number;
}

const PACKAGES = notices.packages as Notice[];
const TEXTS = notices.texts as string[];

function Row({ notice }: { notice: Notice }) {
  const { t } = useTranslation("mobile");
  const [open, setOpen] = useState(false);
  const text = notice.text >= 0 ? TEXTS[notice.text] : undefined;
  return (
    <View className="border-b border-line px-4 py-3">
      <Pressable
        testID={`license-${notice.name}`}
        accessibilityRole="button"
        accessibilityState={{ expanded: open }}
        onPress={() => setOpen(!open)}
        className="flex-row items-center gap-2"
      >
        <Text className="flex-1 font-bold text-ink" numberOfLines={1}>
          {notice.name}
        </Text>
        <Text className="text-xs text-ink-soft">
          {notice.version} · {notice.license}
        </Text>
      </Pressable>
      {open && (
        <View className="gap-2 pt-2">
          {notice.url ? (
            <Text
              accessibilityRole="link"
              className="text-xs font-bold text-brand"
              onPress={() => void Linking.openURL(notice.url)}
            >
              {notice.url}
            </Text>
          ) : null}
          <Text selectable className="font-mono text-[11px] text-ink-soft">
            {text ?? t("licenses.noText", { license: notice.license })}
          </Text>
        </View>
      )}
    </View>
  );
}

/**
 * The open-source software in the app and the licences it comes under:
 * every bundled JavaScript package and the native libraries of the app's
 * own modules, written at build time (`scripts/licenses.js`).
 */
export function LicensesScreen() {
  const { t } = useTranslation("mobile");
  const { colors } = useTheme();
  const [query, setQuery] = useState("");
  const needle = query.trim().toLowerCase();
  const shown = needle ? PACKAGES.filter((notice) => notice.name.includes(needle)) : PACKAGES;
  return (
    <SafeAreaView edges={["bottom"]} className="flex-1 bg-bg" testID="licenses-screen">
      <View className="gap-2 px-4 pt-3 pb-2">
        <Text className="text-sm text-ink-soft">
          {t("licenses.lead", { count: PACKAGES.length })}
        </Text>
        <TextInput
          testID="licenses-search"
          value={query}
          onChangeText={setQuery}
          autoCapitalize="none"
          placeholder={t("licenses.search")}
          placeholderTextColor={colors["ink-faint"]}
          className="rounded-control bg-surface px-4 py-2.5 text-base text-ink"
        />
      </View>
      <FlashList
        data={shown}
        keyExtractor={(notice) => `${notice.name}@${notice.version}`}
        renderItem={({ item }) => <Row notice={item} />}
      />
    </SafeAreaView>
  );
}
