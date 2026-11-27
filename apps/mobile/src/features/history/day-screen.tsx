import { useMutation, useQueryClient } from "@tanstack/react-query";
import { errorReason, type geo } from "@tokuly/api";
import { compactDuration, formatDistance, kmh } from "@tokuly/core/format";
import {
  activityMeta,
  buildTimeline,
  decodePolyline,
  historyKeys,
  parseDay,
  shiftDay,
  toTrack,
  visitEmoji,
} from "@tokuly/core/history";
import { toLatLng } from "@tokuly/core/places";
import { router, Stack } from "expo-router";
import { ChevronLeft, ChevronRight } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Text, View } from "react-native";
import { Button } from "#/components/button";
import { IconButton } from "#/components/icon-button";
import { EmptyState, ErrorNote, Page, Spinner } from "#/components/page";
import { TextField } from "#/components/text-field";
import { errorMessage } from "#/lib/errors";
import { sticker, useTheme } from "#/lib/theme";
import { asAccount, historyClient, NS, useAccountToday, useDay, useRoute } from "./data";
import { DayMap, type DayStop } from "./day-map";
import { dayTitle, Faces, Fact, GazetteerCredit, VisitCard, VisitEmoji } from "./parts";

/** `/history/<YYYY-MM-DD>`: one day as a diary — where the person was, how they got around, what they want to remember. */
export function DayScreen({ date }: { date: string }) {
  const { t, i18n } = useTranslation(NS);
  const { colors } = useTheme();
  const today = useAccountToday();
  const day = useDay(date);
  const route = useRoute(date);
  const valid = parseDay(date) !== null;
  const title = valid ? dayTitle(date, i18n.language) : t("title");

  const header = (
    <>
      <Stack.Screen options={{ title }} />
      <View className="flex-row items-center justify-between">
        <IconButton
          testID="history-day-previous"
          label={t("day.previous")}
          icon={<ChevronLeft size={20} color={colors.ink} />}
          onPress={() => router.replace(`/history/${shiftDay(date, -1)}`)}
        />
        <View className={date < today ? "" : "opacity-30"}>
          <IconButton
            testID="history-day-next"
            label={t("day.next")}
            icon={<ChevronRight size={20} color={colors.ink} />}
            onPress={() => date < today && router.replace(`/history/${shiftDay(date, 1)}`)}
          />
        </View>
      </View>
    </>
  );

  if (!valid) {
    return (
      <Page testID="history-day">
        <Stack.Screen options={{ title }} />
        <EmptyState emoji="🗓️" title={t("day.invalid")} />
      </Page>
    );
  }
  if (day.isPending || route.isPending) {
    return (
      <Page testID="history-day">
        {header}
        <Spinner />
      </Page>
    );
  }
  if (day.isError) {
    return (
      <Page testID="history-day">
        {header}
        <ErrorNote>{errorMessage(day.error, { namespace: NS })}</ErrorNote>
      </Page>
    );
  }

  const summary = day.data.summary;
  const visits = day.data.visits;
  const track = toTrack(route.data?.points ?? []);
  const encoded = decodePolyline(day.data.pathPolyline);
  // The replay points draw the finer line; the server's simplified path stands in without them.
  const path = track.length > 1 ? track.map(({ lat, lng }) => ({ lat, lng })) : encoded;
  const timeline = buildTimeline(visits, track);
  // Visits are numbered in the order of the day, the same on the map and in the list.
  const orderOf = new Map(
    timeline
      .flatMap((item) => (item.kind === "visit" ? [item.visit.id] : []))
      .map((id, index) => [id, index + 1]),
  );
  const stops: DayStop[] = visits.flatMap((visit) => {
    const position = toLatLng(visit.center);
    return position
      ? [{ id: visit.id, position, order: orderOf.get(visit.id) ?? 0, emoji: visitEmoji(visit) }]
      : [];
  });
  const hasRoute = path.length > 1;
  const empty = visits.length === 0 && !hasRoute && (summary?.distanceMeters ?? 0) < 1;
  const routeMissing =
    !hasRoute && errorReason(route.error) === "HISTORY_UNAVAILABLE" && visits.length > 0;

  return (
    <Page testID="history-day">
      {header}
      {empty ? (
        <EmptyState
          emoji="🛋️"
          title={t("day.emptyTitle")}
          body={date === today ? t("day.emptyToday") : t("day.emptyBody")}
        />
      ) : (
        <>
          <DayMap path={path} stops={stops} />
          {routeMissing && (
            <Text className="px-1 text-sm font-bold text-ink-soft">{t("day.routeMissing")}</Text>
          )}
          <View testID="history-day-facts" className="flex-row flex-wrap gap-2">
            <Fact emoji="👟" text={formatDistance(summary?.distanceMeters ?? 0)} />
            <Fact
              emoji="📍"
              text={t("day.visits", { count: summary?.visitCount ?? visits.length })}
            />
            {(summary?.newCellCount ?? 0) > 0 && (
              <Fact
                emoji="✨"
                text={t("day.newCells", { count: summary?.newCellCount })}
                tint="bg-lime"
              />
            )}
            {day.data.maxSpeedMps > 0.5 && (
              <Fact emoji="💨" text={t("day.maxSpeed", { speed: kmh(day.data.maxSpeedMps) })} />
            )}
            {summary?.topLocality ? <Fact emoji="🗾" text={summary.topLocality} /> : null}
          </View>
          {day.data.friendsMet.length > 0 && (
            <View className="gap-2 rounded-card bg-lime p-4" style={sticker(colors)}>
              <Text className="font-black text-[#17122b]">
                {t("day.friendsMetCount", { count: day.data.friendsMet.length })}
              </Text>
              <Faces users={day.data.friendsMet} />
            </View>
          )}
          <View className="gap-3">
            <Text className="px-1 text-lg font-black text-ink">{t("day.timeline")}</Text>
            {visits.length === 0 ? (
              <Text className="px-1 text-sm text-ink-soft">{t("day.noVisits")}</Text>
            ) : (
              timeline.map((item) => {
                if (item.kind === "move") {
                  const meta = activityMeta(item.activity);
                  return (
                    <View
                      key={`move-${item.from}`}
                      className="ml-5 border-l-[3px] border-dashed border-line py-1 pl-6"
                    >
                      <Text className="text-sm font-bold text-ink-soft">
                        {meta.emoji} {t(`activity.${meta.key}`)}・
                        {formatDistance(item.distanceMeters)}・
                        {compactDuration(i18n.language, item.to - item.from)}
                      </Text>
                    </View>
                  );
                }
                return (
                  <VisitCard
                    key={item.visit.id}
                    visit={item.visit}
                    leading={<VisitEmoji visit={item.visit} order={orderOf.get(item.visit.id)} />}
                  />
                );
              })
            )}
          </View>
        </>
      )}
      <DayMemo
        key={`${date}:${day.data.memo?.updatedAt?.seconds ?? 0}`}
        date={date}
        memo={day.data.memo}
      />
      {!empty && <GazetteerCredit />}
      {!empty && (
        <Button
          testID="history-day-delete"
          variant="ghost"
          onPress={() => router.push(`/history/delete?date=${date}`)}
        >
          {t("delete.dayAction")}
        </Button>
      )}
    </Page>
  );
}

const MAX_TEXT = 2000;

/**
 * The day's diary entry: a few words. Its photos are kept as they are
 * (added on the web); saving it empty without photos removes it.
 */
function DayMemo({ date, memo }: { date: string; memo: geo.DayMemo | undefined }) {
  const { t } = useTranslation(NS);
  const queryClient = useQueryClient();
  const saved = memo?.text ?? "";
  const [text, setText] = useState(saved);
  const save = useMutation({
    mutationFn: () =>
      asAccount(() =>
        historyClient().setDayMemo({
          date,
          text: text.trim(),
          photoAssetIds: (memo?.photos ?? []).map((photo) => photo.assetId),
        }),
      ),
    onSuccess: (answer) => {
      if (!answer) return;
      setText(answer.memo?.text ?? "");
      void queryClient.invalidateQueries({ queryKey: historyKeys.all });
      Alert.alert(
        answer.memo?.text || answer.memo?.photos.length ? t("memo.saved") : t("memo.removed"),
      );
    },
    onError: (error) => Alert.alert(errorMessage(error, { namespace: NS })),
  });
  const photos = memo?.photos.length ?? 0;
  return (
    <View className="gap-2">
      <TextField
        testID="history-memo"
        label={t("memo.title")}
        placeholder={t("memo.placeholder")}
        value={text}
        maxLength={MAX_TEXT}
        multiline
        onChangeText={setText}
      />
      {photos > 0 && (
        <Text className="px-1 text-xs text-ink-faint">
          {t("mobile:history.memoPhotos", { count: photos })}
        </Text>
      )}
      {text.trim() !== saved.trim() && (
        <Button testID="history-memo-save" loading={save.isPending} onPress={() => save.mutate()}>
          {t("memo.save")}
        </Button>
      )}
    </View>
  );
}
