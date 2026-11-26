import { shorts } from "@tokuly/api";
import { AssetFailedError } from "@tokuly/core/errors";
import { MAX_SECONDS } from "@tokuly/core/shorts";
import { router } from "expo-router";
import { useTranslation } from "react-i18next";
import { Text, View } from "react-native";
import { Button } from "#/components/button";
import { EmptyState, Spinner } from "#/components/page";
import { PostsBanner } from "#/features/media/posts-banner";
import { useMe } from "#/features/session/store";
import { errorMessage } from "#/lib/errors";
import { useMyShorts } from "./data";
import { ShortGrid, watch } from "./grid";

const { ShortStatus } = shorts;

/**
 * The creator's own shorts in every state, newest first: the ones on their
 * way up first, then drafts being processed (looked at again until they go
 * live), the published ones, and failed ones with the server's reason.
 */
export function MyShortsScreen() {
  const { t } = useTranslation("shorts");
  const myId = useMe()?.user?.id ?? "";
  const list = useMyShorts();
  const items = list.data?.pages.flatMap((page) => page.shorts) ?? [];
  const failed = items.filter((short) => short.status === ShortStatus.FAILED);
  const statusOf = (short: shorts.Short) =>
    short.status === ShortStatus.PROCESSING
      ? t("status.processing")
      : short.status === ShortStatus.FAILED
        ? t("status.failed")
        : undefined;
  if (list.isPending) return <Spinner />;
  return (
    <View className="flex-1 bg-bg" testID="my-shorts">
      <ShortGrid
        shorts={items}
        statusOf={statusOf}
        onEnd={() => list.hasNextPage && void list.fetchNextPage()}
        onPress={(short) =>
          short.status === ShortStatus.PUBLISHED
            ? watch({ kind: "user", key: myId }, short)
            : undefined
        }
        header={
          <View className="gap-3 pt-3">
            <View className="px-3">
              <Button testID="shorts-post" onPress={() => router.push("/shorts/new")}>
                {t("bar.post")}
              </Button>
            </View>
            <PostsBanner kind="short" />
            {failed.map((short) => (
              <Text
                key={short.id}
                accessibilityRole="alert"
                className="mx-3 rounded-control bg-danger/10 px-4 py-2 text-sm font-bold text-danger"
              >
                {short.caption || t("studio.noCaption")}:{" "}
                {errorMessage(
                  new AssetFailedError(
                    short.studio?.processingError || "PROCESSING_FAILED",
                    MAX_SECONDS,
                  ),
                  {
                    namespace: "shorts",
                  },
                )}
              </Text>
            ))}
          </View>
        }
        empty={<EmptyState emoji="🎬" title={t("studio.empty")} body={t("studio.emptyBody")} />}
      />
    </View>
  );
}
