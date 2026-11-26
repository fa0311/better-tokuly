import { router } from "expo-router";
import { useTranslation } from "react-i18next";
import { Text, View } from "react-native";
import { Button } from "#/components/button";
import { EmptyState } from "#/components/page";
import { webScreenHref } from "#/features/web/handoff";
import { useAntenna, useList } from "./data";
import { Timeline } from "./timeline";

/** A note list's timeline. Its members and settings are edited on the web. */
export function ListScreen({ listId }: { listId: string }) {
  const { t } = useTranslation("notelists");
  const list = useList(listId);
  if (list.isError || list.data === null)
    return <EmptyState emoji="📋" title={t("list.notFound")} />;
  return (
    <View className="flex-1 bg-bg" testID="list-screen">
      <Timeline
        source={{ kind: "list", listId }}
        empty={{ emoji: "📋", title: t("list.emptyTimeline"), body: t("list.emptyTimelineBody") }}
        header={
          list.data ? (
            <Header
              title={list.data.name}
              line={t("lists.members", { count: list.data.memberCount })}
              webPath={`/notes/lists/${listId}`}
            />
          ) : undefined
        }
      />
    </View>
  );
}

/** What one of the viewer's antennas collected. It is set up on the web. */
export function AntennaScreen({ antennaId }: { antennaId: string }) {
  const { t } = useTranslation("notelists");
  const antenna = useAntenna(antennaId);
  if (antenna.isError || antenna.data === null)
    return <EmptyState emoji="📡" title={t("antennas.notFound")} />;
  return (
    <View className="flex-1 bg-bg" testID="antenna-screen">
      <Timeline
        source={{ kind: "antenna", antennaId }}
        empty={{
          emoji: "📡",
          title: t("antennas.emptyTimeline"),
          body: t("antennas.emptyTimelineBody"),
        }}
        header={
          antenna.data ? (
            <Header title={antenna.data.name} webPath={`/notes/antennas/${antennaId}`} />
          ) : undefined
        }
      />
    </View>
  );
}

function Header({ title, line, webPath }: { title: string; line?: string; webPath: string }) {
  const { t } = useTranslation("notelists");
  return (
    <View className="flex-row items-center gap-3 px-4 py-3">
      <View className="flex-1">
        <Text className="text-lg font-black text-ink" numberOfLines={1}>
          {title}
        </Text>
        {line && <Text className="text-xs text-ink-soft">{line}</Text>}
      </View>
      <Button
        variant="secondary"
        size="sm"
        onPress={() => router.push(webScreenHref(webPath) as never)}
      >
        {t("list.edit")}
      </Button>
    </View>
  );
}
