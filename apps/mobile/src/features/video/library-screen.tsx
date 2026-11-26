import { router } from "expo-router";
import { Clapperboard, History, Tv } from "lucide-react-native";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Page, Section, Spinner } from "#/components/page";
import { webScreenHref } from "#/features/web/handoff";
import { useTheme } from "#/lib/theme";
import { useMyPlaylists } from "./data";
import { PlaylistRow } from "./playlist-row";

/**
 * The viewer's own corner: watch later, liked videos and their playlists,
 * the history, their subscriptions — and the studio, which stays on the web
 * (uploading long videos, editing, analytics).
 */
export function LibraryScreen() {
  const { t } = useTranslation("video");
  const { colors } = useTheme();
  const playlists = useMyPlaylists();
  return (
    <Page testID="video-library-screen">
      <Section>
        <Row
          testID="library-history"
          icon={<History color={colors.ink} size={22} />}
          label={t("history.title")}
          onPress={() => router.push("/watch/history")}
        />
        <Row
          testID="library-subscriptions"
          icon={<Tv color={colors.ink} size={22} />}
          label={t("subscriptions.title")}
          onPress={() => router.push("/watch/channels")}
        />
        <Row
          testID="library-studio"
          icon={<Clapperboard color={colors.ink} size={22} />}
          label={t("app.studio")}
          hint={t("app.studioHint")}
          onPress={() => router.push(webScreenHref("/studio") as never)}
        />
      </Section>
      <Section title={t("playlist.libraryTitle")}>
        {playlists.isPending ? (
          <Spinner />
        ) : (
          <View className="py-2">
            {playlists.data?.map((playlist) => (
              <PlaylistRow key={playlist.id} playlist={playlist} />
            ))}
          </View>
        )}
      </Section>
      <Text className="px-1 text-xs text-ink-soft">{t("playlist.libraryHint")}</Text>
    </Page>
  );
}

function Row({
  testID,
  icon,
  label,
  hint,
  onPress,
}: {
  testID: string;
  icon: ReactNode;
  label: string;
  hint?: string;
  onPress(): void;
}) {
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      onPress={onPress}
      className="flex-row items-center gap-3 px-4 py-3.5 active:bg-surface-2"
    >
      {icon}
      <View className="flex-1">
        <Text className="text-base font-black text-ink">{label}</Text>
        {hint ? <Text className="text-xs text-ink-soft">{hint}</Text> : null}
      </View>
    </Pressable>
  );
}
