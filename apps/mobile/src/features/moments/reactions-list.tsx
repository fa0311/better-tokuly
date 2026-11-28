import { FlashList } from "@shopify/flash-list";
import { errorReason, type moments } from "@tokuly/api";
import { realmojiGlyph } from "@tokuly/core/moments";
import { X } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Modal, Pressable, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { EmptyState, ErrorNote, Spinner } from "#/components/page";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";
import { useMomentReactions } from "./data";
import { MomentReactionFace } from "./reactions";

const ACCESS_ERRORS = new Set(["MOMENT_LOCKED", "MOMENT_NOT_FOUND"]);

/** The people and reaction types on a Moment the caller may see. */
export function MomentReactionsSheet({
  moment,
  visible,
  onClose,
}: {
  moment: moments.Moment;
  visible: boolean;
  onClose(): void;
}) {
  const { t } = useTranslation("moments");
  const insets = useSafeAreaInsets();
  const { colors } = useTheme();
  const reactions = useMomentReactions(moment.id, visible && !moment.locked);
  const rows = reactions.data?.pages.flatMap((page) => page.reactions) ?? [];
  const accessRevoked = ACCESS_ERRORS.has(errorReason(reactions.error) ?? "");
  const hideCachedRows = accessRevoked || (reactions.isError && !reactions.isFetchNextPageError);
  const loadingFirstPage =
    reactions.isPending || (reactions.isFetching && !reactions.isFetchingNextPage);
  const visibleRows = hideCachedRows ? [] : rows;
  const footer = reactions.isFetchingNextPage ? (
    <Spinner />
  ) : reactions.isFetchNextPageError ? (
    <View className="gap-3 px-4 py-3">
      <ErrorNote>{errorMessage(reactions.error)}</ErrorNote>
      <Button
        variant="secondary"
        onPress={() => void reactions.fetchNextPage()}
        accessibilityLabel={t("composer.retry")}
      >
        {t("composer.retry")}
      </Button>
    </View>
  ) : null;

  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={onClose}>
      <Pressable accessibilityRole="none" className="flex-1 bg-black/40" onPress={onClose} />
      <View
        testID="moment-reactions-sheet"
        className="h-[68%] rounded-t-3xl bg-surface pt-4"
        style={{ paddingBottom: insets.bottom }}
      >
        <View className="flex-row items-center justify-between px-5 pb-2">
          <View>
            <Text accessibilityRole="header" className="text-lg font-black text-ink">
              {t("reactions.title")}
            </Text>
            <Text className="text-sm text-ink-soft">
              {t("card.reactions", { count: moment.reactionCount })}
            </Text>
          </View>
          <Pressable
            testID="moment-reactions-close"
            accessibilityRole="button"
            accessibilityLabel={t("reactions.close")}
            onPress={onClose}
            className="items-center justify-center rounded-full bg-surface-2 p-2"
          >
            <X size={20} color={colors.ink} />
          </Pressable>
        </View>
        {loadingFirstPage ? (
          <View className="flex-1 justify-center">
            <Spinner />
          </View>
        ) : hideCachedRows ? (
          <View className="gap-3 px-4 py-3">
            <ErrorNote>{errorMessage(reactions.error)}</ErrorNote>
            <Button
              variant="secondary"
              onPress={() => void reactions.refetch()}
              accessibilityLabel={t("composer.retry")}
            >
              {t("composer.retry")}
            </Button>
          </View>
        ) : visibleRows.length === 0 ? (
          <EmptyState emoji="💬" title={t("reactions.empty")} />
        ) : (
          <FlashList
            className="flex-1"
            data={visibleRows}
            keyExtractor={(reaction) => reaction.id}
            onEndReached={() => {
              if (
                reactions.hasNextPage &&
                !reactions.isFetchingNextPage &&
                !reactions.isFetchNextPageError
              )
                void reactions.fetchNextPage();
            }}
            onEndReachedThreshold={0.4}
            ListFooterComponent={footer}
            renderItem={({ item }) => <ReactionRow reaction={item} />}
          />
        )}
      </View>
    </Modal>
  );
}

function ReactionRow({ reaction }: { reaction: moments.MomentReaction }) {
  const { t } = useTranslation("moments");
  const name = reaction.user?.displayName || reaction.user?.handle || t("friends.someone");
  const value = reaction.reaction;
  const kind =
    value.case === "realmoji"
      ? realmojiGlyph(value.value.emoji)
      : value.case === "stamp"
        ? value.value.name
        : "";

  return (
    <View
      accessible
      accessibilityRole="text"
      accessibilityLabel={t("reactions.person", { name, reaction: kind })}
      className="flex-row items-center gap-3 px-5 py-2.5"
    >
      <Avatar user={reaction.user} size="md" />
      <Text className="min-w-0 flex-1 font-black text-ink" numberOfLines={1}>
        {name}
      </Text>
      <MomentReactionFace reaction={reaction} size={44} />
    </View>
  );
}
