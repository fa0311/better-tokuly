import type { common, stamp } from "@tokuly/api";
import { Image } from "expo-image";
import { router } from "expo-router";
import { Clock, Heart, Smile, Store } from "lucide-react-native";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { FlatList, Modal, Pressable, Text, useWindowDimensions, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { Button } from "#/components/button";
import { EmptyState, Spinner } from "#/components/page";
import { mediaUrl } from "#/lib/config";
import { useTheme } from "#/lib/theme";
import { NS, useLibrary, usePack } from "./data";
import { StampImage } from "./stamp-image";

const COLUMNS = 4;
const TAB = 44;

type Shelf = "recents" | "favorites" | "personal" | `pack:${string}`;

/**
 * Choosing a stamp to send, from the bottom of the screen: the ones used
 * last, the favourites, the person's own and each installed pack, in the
 * person's order. A tap picks the stamp and closes the sheet. Buying and
 * making stamps are the store's and the web's.
 */
export function StampPicker({
  visible,
  onPick,
  onClose,
}: {
  visible: boolean;
  onPick(stamp: common.StampRef): void;
  onClose(): void;
}) {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  const { height } = useWindowDimensions();
  const library = useLibrary(visible);
  const [shelf, setShelf] = useState<Shelf>("recents");
  const data = library.data;
  const toStore = () => {
    onClose();
    router.push("/stamps");
  };

  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={onClose}>
      <Pressable accessibilityRole="none" className="flex-1 bg-black/40" onPress={onClose} />
      <View
        testID="stamp-picker"
        className="rounded-t-3xl bg-surface pt-3"
        style={{ height: Math.round(height * 0.55), paddingBottom: insets.bottom }}
      >
        <View className="flex-row items-center justify-between px-4 pb-2">
          <Text className="text-base font-black text-ink">{t("picker.title")}</Text>
          <Pressable
            testID="stamp-picker-store"
            accessibilityRole="button"
            onPress={toStore}
            className="flex-row items-center gap-1 rounded-full bg-brand-soft px-3 py-1.5"
          >
            <Store size={16} color={colors["brand-strong"]} />
            <Text className="text-sm font-black text-brand-strong">{t("picker.store")}</Text>
          </Pressable>
        </View>
        {library.isPending ? (
          <Spinner />
        ) : !data ? (
          <EmptyState emoji="😵" title={t("store.error")} />
        ) : (
          <>
            <ShelfTabs library={data} shelf={shelf} onChange={setShelf} />
            <View className="flex-1">
              <ShelfStamps
                library={data}
                shelf={shelf}
                onPick={(picked) => {
                  onClose();
                  onPick(picked);
                }}
                toStore={toStore}
              />
            </View>
          </>
        )}
      </View>
    </Modal>
  );
}

function ShelfTabs({
  library,
  shelf,
  onChange,
}: {
  library: stamp.GetLibraryResponse;
  shelf: Shelf;
  onChange(shelf: Shelf): void;
}) {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const tab = (value: Shelf, label: string, icon: ReactNode) => (
    <Pressable
      key={value}
      testID={`stamp-shelf-${value}`}
      accessibilityRole="tab"
      accessibilityLabel={label}
      accessibilityState={{ selected: shelf === value }}
      onPress={() => onChange(value)}
      style={{ width: TAB, height: TAB }}
      className={`items-center justify-center rounded-2xl ${shelf === value ? "bg-brand-soft" : ""}`}
    >
      {icon}
    </Pressable>
  );
  return (
    <FlatList
      horizontal
      accessibilityRole="tablist"
      accessibilityLabel={t("picker.tabs")}
      showsHorizontalScrollIndicator={false}
      contentContainerClassName="gap-1 px-3 pb-2"
      className="grow-0 border-b border-line"
      data={library.installed}
      keyExtractor={(pack) => pack.id}
      ListHeaderComponent={
        <View className="flex-row gap-1">
          {tab("recents", t("picker.recents"), <Clock size={22} color={colors.ink} />)}
          {tab("favorites", t("picker.favorites"), <Heart size={22} color={colors.ink} />)}
          {library.personal.length > 0 &&
            tab("personal", t("picker.personal"), <Smile size={22} color={colors.ink} />)}
        </View>
      }
      renderItem={({ item: pack }) =>
        tab(
          `pack:${pack.id}`,
          pack.name,
          pack.cover?.url ? (
            <Image
              source={mediaUrl(pack.cover.url)}
              contentFit="contain"
              style={{ width: 32, height: 32 }}
            />
          ) : (
            <Text className="font-black text-ink">{pack.name.slice(0, 1)}</Text>
          ),
        )
      }
    />
  );
}

function ShelfStamps({
  library,
  shelf,
  onPick,
  toStore,
}: {
  library: stamp.GetLibraryResponse;
  shelf: Shelf;
  onPick(stamp: common.StampRef): void;
  toStore(): void;
}) {
  const { t } = useTranslation(NS);
  const packId = shelf.startsWith("pack:") ? shelf.slice("pack:".length) : "";
  const pack = usePack(packId);
  if (packId && pack.isPending) return <Spinner />;
  const stamps =
    shelf === "recents"
      ? library.recents
      : shelf === "favorites"
        ? library.favorites
        : shelf === "personal"
          ? library.personal
          : (pack.data?.stamps ?? []);
  const usable = stamps.filter((item) => item.usable && item.stampRef);
  if (usable.length === 0) {
    const noPacks = library.installed.length === 0;
    const [title, body] =
      shelf === "recents"
        ? [
            t("picker.recentsEmpty"),
            noPacks ? t("picker.noPacksBody") : t("picker.recentsEmptyBody"),
          ]
        : shelf === "favorites"
          ? [t("picker.favoritesEmpty"), t("picker.favoritesEmptyBody")]
          : [t("picker.packEmpty"), undefined];
    return (
      <EmptyState
        emoji={shelf === "favorites" ? "💝" : "🧸"}
        title={title}
        body={body}
        action={
          noPacks ? (
            <Button testID="stamp-picker-empty-store" size="sm" onPress={toStore}>
              {t("library.toStore")}
            </Button>
          ) : undefined
        }
      />
    );
  }
  return <StampGrid stamps={usable} onPress={onPick} />;
}

/** Stamps in rows of four; `onPress` gets the tapped one. */
export function StampGrid({
  stamps,
  onPress,
  header,
}: {
  stamps: stamp.Stamp[];
  onPress?(stamp: common.StampRef): void;
  header?: ReactNode;
}) {
  const { width } = useWindowDimensions();
  const cell = Math.floor((width - 24) / COLUMNS);
  return (
    <FlatList
      data={stamps}
      numColumns={COLUMNS}
      keyExtractor={(item, index) => item.stampRef?.id ?? `${index}`}
      contentContainerClassName="px-3 py-2"
      ListHeaderComponent={header ? <View>{header}</View> : undefined}
      renderItem={({ item }) => (
        <Pressable
          testID={`stamp-${item.stampRef!.id}`}
          accessibilityRole={onPress ? "button" : "image"}
          accessibilityLabel={item.stampRef!.name}
          disabled={!onPress}
          onPress={() => onPress?.(item.stampRef!)}
          style={{ width: cell, height: cell }}
          className="items-center justify-center rounded-2xl p-1.5 active:bg-surface-2"
        >
          <StampImage stamp={item.stampRef!} size={cell - 12} />
        </Pressable>
      )}
    />
  );
}
