import { normalizeHashtag } from "@tokuly/core/notes";
import { router } from "expo-router";
import { useTranslation } from "react-i18next";
import { Text, View } from "react-native";
import { Button } from "#/components/button";
import { EmptyState } from "#/components/page";
import { composeHref } from "./compose-href";
import { useTimeline } from "./data";
import { Timeline } from "./timeline";

/** A hashtag's notes, newest first, with a way to write one with the tag. */
export function TagScreen({ tag: raw }: { tag: string }) {
  const { t } = useTranslation("notes");
  const tag = normalizeHashtag(raw);
  if (!tag) return <EmptyState emoji="🏷️" title={t("tag.invalid")} body={t("tag.invalidBody")} />;
  return <TagTimeline tag={tag} />;
}

function TagTimeline({ tag }: { tag: string }) {
  const { t } = useTranslation("notes");
  const source = { kind: "hashtag", tag } as const;
  // The same query as the timeline's: its first page says how many notes carry the tag.
  const count = Number(useTimeline(source).data?.pages[0]?.hashtag?.noteCount ?? 0n);
  return (
    <View className="flex-1 bg-bg" testID="tag-screen">
      <Timeline
        source={source}
        empty={{ emoji: "🏷️", title: t("tag.empty", { tag }), body: t("tag.emptyBody") }}
        header={
          <View className="flex-row items-center gap-3 px-4 py-3">
            <View className="flex-1">
              <Text className="text-xl font-black text-ink">#{tag}</Text>
              {count > 0 && (
                <Text className="text-xs text-ink-soft">{t("tag.count", { count })}</Text>
              )}
            </View>
            <Button
              testID="tag-post"
              size="sm"
              onPress={() => router.push(composeHref({ text: `#${tag} ` }))}
            >
              {t("tag.post")}
            </Button>
          </View>
        }
      />
    </View>
  );
}
