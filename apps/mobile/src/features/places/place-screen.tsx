import { Camera, Map as MapLibre, Marker } from "@maplibre/maplibre-react-native";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { geo, isNotFound, toDate, toMillis } from "@tokuly/api";
import { compactAge, compactDuration, zonedDate } from "@tokuly/core/format";
import { historyKeys } from "@tokuly/core/history";
import { buildMapStyle } from "@tokuly/core/map";
import { type LatLng, labelMeta, placeKeys, toLatLng, visibilityKey } from "@tokuly/core/places";
import { router, Stack, useNavigation } from "expo-router";
import {
  Bookmark,
  BookmarkCheck,
  MapPinCheck,
  Navigation,
  Pencil,
  Trash2,
} from "lucide-react-native";
import { useRef } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Linking, Platform, Pressable, Text, View } from "react-native";
import { Button } from "#/components/button";
import { EmptyState, ErrorNote, Page, Row, Section, Spinner } from "#/components/page";
import { useAccountTimeZone } from "#/features/history/data";
import { Faces, GazetteerCredit } from "#/features/history/parts";
import { useMe } from "#/features/session/store";
import { webScreenHref } from "#/features/web/handoff";
import { TILE_CONFIG } from "#/lib/config";
import { errorMessage } from "#/lib/errors";
import { INK_ON_BRIGHT, sticker, useTheme } from "#/lib/theme";
import { asAccount, NS, placeClient, usePlace, usePlaceVisits } from "./data";
import { PlaceBubble } from "./parts";

const KNOWN_BADGES = ["GOOD_FOR_GROUPS", "BEST_FRIENDS_HANG_HERE", "LOYAL_REGULARS"];

/** Where the place is in the device's own map app, for the way there. */
function openInMaps(center: LatLng, name: string): void {
  const at = `${center.lat},${center.lng}`;
  const label = encodeURIComponent(name);
  void Linking.openURL(
    Platform.OS === "ios" ? `maps:?ll=${at}&q=${label}` : `geo:${at}?q=${at}(${label})`,
  );
}

/** `/places/<id>`: everything about one place, as far as the server lets the viewer know it. */
export function PlaceScreen({ placeId }: { placeId: string }) {
  const { t, i18n } = useTranslation(NS);
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const navigation = useNavigation();
  const me = useMe();
  const zone = useAccountTimeZone();
  const query = usePlace(placeId);
  const place = query.data?.place;
  const center = toLatLng(place?.center);
  const mine = Boolean(place && place.ownerId === me?.user?.id);
  const visits = usePlaceVisits(placeId, Boolean(place));

  const setSaved = useMutation({
    mutationFn: (saved: boolean) =>
      asAccount(async () => {
        await (saved
          ? placeClient().savePlace({ placeId })
          : placeClient().unsavePlace({ placeId }));
        return true;
      }),
    onSuccess: (answer, saved) => {
      if (!answer) return;
      queryClient.setQueryData(
        placeKeys.place(placeId),
        (current: geo.GetPlaceResponse | undefined) =>
          current?.place ? { ...current, place: { ...current.place, saved } } : current,
      );
      void queryClient.invalidateQueries({ queryKey: placeKeys.saved });
    },
    onError: (error) => Alert.alert(errorMessage(error, { namespace: NS })),
  });
  // A deletion is sent once: the row and its question can be tapped again while it is on its way.
  const deleting = useRef(false);
  const remove = useMutation({
    mutationFn: () => asAccount(() => placeClient().deletePlace({ placeId })),
    onSuccess: (answer) => {
      if (!answer) return;
      // Reset, not marked stale: the place is in no list, search or visit
      // shown from memory, not even for the moment a re-read takes.
      void queryClient.resetQueries({ queryKey: placeKeys.all });
      void queryClient.resetQueries({ queryKey: historyKeys.all });
    },
    onError: (error) => {
      deleting.current = false;
      Alert.alert(errorMessage(error, { namespace: NS }));
    },
  });
  const sendDelete = () => {
    if (deleting.current) return;
    deleting.current = true;
    // Closing is this screen's own: under another screen the answer moves nothing.
    remove.mutate(undefined, {
      onSuccess: (answer) => answer && navigation.isFocused() && router.back(),
    });
  };
  const confirmDelete = () => {
    if (deleting.current) return;
    Alert.alert(t("place.deleteTitle", { name: place?.name ?? "" }), t("place.deleteBody"), [
      { text: t("common:actions.cancel"), style: "cancel" },
      { text: t("place.delete"), style: "destructive", onPress: sendDelete },
    ]);
  };

  if (query.isPending) return <Spinner />;
  // Not read (offline, the server down) is not "gone".
  if (!place && query.isError && !isNotFound(query.error)) {
    return (
      <Page testID="place">
        <ErrorNote>{errorMessage(query.error, { namespace: NS })}</ErrorNote>
      </Page>
    );
  }
  if (!place || !center) {
    return (
      <Page testID="place">
        <EmptyState emoji="🧭" title={t("place.notFound")} body={t("place.notFoundBody")} />
      </Page>
    );
  }
  const meta = labelMeta(place.label);
  const lastVisit = toMillis(place.myLastVisitAt);
  const badges = (query.data?.badges ?? []).filter((badge) => KNOWN_BADGES.includes(badge));
  const chips = [
    place.label !== geo.PlaceLabel.CUSTOM && place.label !== geo.PlaceLabel.UNSPECIFIED
      ? t(`labels.${meta.key}`)
      : "",
    mine ? t(`visibility.${visibilityKey(place.visibility)}`) : "",
    place.category ? t(`categories.${place.category}`, { defaultValue: place.category }) : "",
    place.autoDetected ? t("place.autoDetected") : "",
  ].filter(Boolean);
  const shared = visibilityKey(place.visibility) !== "private";
  const regulars = query.data?.regulars ?? [];
  const friendsVisited = query.data?.friendsVisited ?? [];

  return (
    <Page testID="place">
      <Stack.Screen options={{ title: place.name }} />
      <PlaceMap place={place} center={center} />
      <View className="gap-4 rounded-card bg-surface p-5" style={sticker(colors)}>
        <View className="flex-row items-start gap-4">
          <PlaceBubble place={place} size="lg" />
          <View className="min-w-0 flex-1 gap-2">
            <Text className="text-2xl font-black text-ink">{place.name}</Text>
            <View className="flex-row flex-wrap gap-1.5">
              {chips.map((chip) => (
                <Text
                  key={chip}
                  className="overflow-hidden rounded-full bg-surface-2 px-2.5 py-1 text-xs font-bold text-ink-soft"
                >
                  {chip}
                </Text>
              ))}
            </View>
          </View>
        </View>
        {place.address !== "" && (
          <View>
            <Text testID="place-address" className="font-bold text-ink">
              📍 {place.address}
            </Text>
            <GazetteerCredit />
          </View>
        )}
        <View className="flex-row flex-wrap gap-2">
          <Button
            testID="place-check-in"
            variant="lime"
            icon={<MapPinCheck size={16} color={INK_ON_BRIGHT} />}
            onPress={() => router.push(`/places/${placeId}/check-in`)}
          >
            {t("actions.checkIn")}
          </Button>
          {!mine && (
            <Button
              testID="place-save"
              variant="secondary"
              loading={setSaved.isPending}
              icon={
                place.saved ? (
                  <BookmarkCheck size={16} color={colors.ink} />
                ) : (
                  <Bookmark size={16} color={colors.ink} />
                )
              }
              onPress={() => setSaved.mutate(!place.saved)}
            >
              {place.saved ? t("place.saved") : t("place.save")}
            </Button>
          )}
          <Button
            testID="place-directions"
            variant="secondary"
            icon={<Navigation size={16} color={colors.ink} />}
            onPress={() => openInMaps(center, place.name)}
          >
            {t("place.directions")}
          </Button>
        </View>
        {badges.length > 0 && (
          <View className="flex-row flex-wrap gap-2">
            {badges.map((badge) => (
              <Text
                key={badge}
                className="overflow-hidden rounded-full bg-sun px-3 py-1 text-xs font-black"
                style={{ color: INK_ON_BRIGHT }}
              >
                {t(`badges.${badge}`)}
              </Text>
            ))}
          </View>
        )}
      </View>

      <View
        testID="place-my-visits"
        className="gap-2 rounded-card bg-surface p-5"
        style={sticker(colors)}
      >
        <Text className="font-black text-ink">{t("place.myVisits")}</Text>
        {place.myVisitCount === 0 ? (
          <Text className="text-sm text-ink-soft">{t("place.noVisits")}</Text>
        ) : (
          <>
            <Text className="text-sm font-bold text-ink-soft">
              {t("place.visitCount", { count: place.myVisitCount })}
              {lastVisit > 0 && ` · ${t("row.lastVisit", { age: compactAge(i18n, lastVisit) })}`}
            </Text>
            {(visits.data?.pages[0]?.visits ?? []).slice(0, 5).map((visit) => {
              const arrived = toDate(visit.arrivedAt);
              if (!arrived) return null;
              return (
                <Pressable
                  key={visit.id}
                  accessibilityRole="button"
                  onPress={() => router.push(`/history/${zonedDate(arrived.getTime(), zone)}`)}
                  className="flex-row items-center gap-3 rounded-2xl px-2 py-2 active:bg-surface-2"
                >
                  <Text>{visit.checkIn ? "✅" : "👣"}</Text>
                  <Text numberOfLines={1} className="flex-1 font-bold text-ink">
                    {arrived.toLocaleString(i18n.language, {
                      timeZone: zone,
                      month: "short",
                      day: "numeric",
                      weekday: "short",
                      hour: "2-digit",
                      minute: "2-digit",
                    })}
                  </Text>
                  <Text className="text-sm text-ink-soft">
                    {visit.departedAt
                      ? compactDuration(i18n.language, Number(visit.durationSeconds) * 1000)
                      : visit.checkIn && Number(visit.durationSeconds) === 0
                        ? t("place.checkedIn")
                        : t("place.now")}
                  </Text>
                </Pressable>
              );
            })}
          </>
        )}
      </View>

      {(regulars.length > 0 || friendsVisited.length > 0) && (
        <View
          testID="place-regulars"
          className="gap-2 rounded-card bg-surface p-5"
          style={sticker(colors)}
        >
          <Text className="font-black text-ink">{t("place.regulars")}</Text>
          <Text className="text-sm font-bold text-ink-soft">
            {t("place.totalVisits", { count: query.data?.totalVisitCount ?? 0 })}
          </Text>
          {regulars.map((regular) =>
            regular.user ? (
              <View key={regular.user.id} className="flex-row items-center gap-3 py-1">
                <Text className="w-6 text-center font-black text-ink">{regular.rank}</Text>
                <Faces users={[regular.user]} />
                <Text numberOfLines={1} className="flex-1 font-bold text-ink">
                  {regular.user.displayName}
                </Text>
                <Text className="text-sm text-ink-soft">
                  {t("row.visits", { count: regular.visitCount })}
                </Text>
              </View>
            ) : null,
          )}
          {friendsVisited.length > 0 && (
            <View className="flex-row items-center gap-2">
              <Faces users={friendsVisited.slice(0, 6)} size="xs" />
              <Text className="text-sm font-bold text-ink-soft">
                {t("place.friendsVisited", { count: friendsVisited.length })}
              </Text>
            </View>
          )}
        </View>
      )}

      <Section>
        {mine && (
          <Row
            testID="place-edit"
            icon={<Pencil size={18} color={colors.ink} />}
            label={t("mobile:places.editOnWeb")}
            onPress={() => router.push(webScreenHref(`/places/${placeId}/edit`) as never)}
          />
        )}
        {shared && (
          <Row
            testID="place-talk"
            label={t("mobile:places.talkOnWeb")}
            onPress={() => router.push(webScreenHref(`/places/${placeId}`) as never)}
          />
        )}
        {mine && (
          <Row
            testID="place-delete"
            icon={<Trash2 size={18} color={colors.danger} />}
            label={t("place.delete")}
            danger
            onPress={confirmDelete}
          />
        )}
      </Section>
    </Page>
  );
}

/**
 * The place on the app's map, as a still picture with its bubble. Only the
 * place itself is on it — no one's position. The map cannot be touched, so
 * the data's credit is written on it.
 */
function PlaceMap({ place, center }: { place: geo.Place; center: LatLng }) {
  const theme = useTheme();
  const { i18n } = useTranslation();
  const {
    sky: _sky,
    projection: _projection,
    ...style
  } = buildMapStyle("tokuly", theme.dark, i18n.language, TILE_CONFIG);
  return (
    <View
      testID="place-map"
      pointerEvents="none"
      className="h-48 overflow-hidden rounded-card"
      style={sticker(theme.colors)}
    >
      <MapLibre
        style={{ flex: 1 }}
        mapStyle={style}
        logo={false}
        attribution={false}
        compass={false}
        dragPan={false}
        touchZoom={false}
        touchRotate={false}
        touchPitch={false}
      >
        <Camera initialViewState={{ center: [center.lng, center.lat], zoom: 15 }} />
        <Marker id="place" lngLat={[center.lng, center.lat]} anchor="bottom">
          <PlaceBubble place={place} />
        </Marker>
      </MapLibre>
      <Text className="absolute right-2 bottom-1 text-[10px] text-ink-soft">© OpenStreetMap</Text>
    </View>
  );
}
