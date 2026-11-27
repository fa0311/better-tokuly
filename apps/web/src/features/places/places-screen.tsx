import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { common, geo } from "@tokuly/api";
import {
  LABELS,
  labelMeta,
  placeQueries,
  shownSuggestions,
  suggestionMark,
} from "@tokuly/core/places";
import { MapPinCheck, Plus, Search } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { errorMessage } from "#/lib/errors";
import { compactDuration, formatDistance } from "#/lib/format";
import { useMe } from "#/stores/session";
import { currentPosition, type LatLng, placeClient, placeKeys, toLatLng } from "./api";
import { CheckInDialog, type CheckInTarget } from "./check-in-dialog";
import { GazetteerCredit } from "./credit";
import { dismissSuggestion, readDismissed } from "./dismissed";
import { PlaceRow } from "./place-row";

export type PlacesTab = "mine" | "saved" | "search";

/** `/places`: my places, the ones I saved, and the search for more. */
export function PlacesScreen({ tab, query }: { tab: PlacesTab; query: string }) {
  const { t } = useTranslation("places");
  const navigate = useNavigate();
  const [checkIn, setCheckIn] = useState<{ open: boolean; target?: CheckInTarget }>({
    open: false,
  });
  const go = (next: PlacesTab) =>
    void navigate({ to: "/places", search: next === "mine" ? {} : { tab: next }, replace: true });

  return (
    <Page title={t("title")} back="/me">
      <div className="flex flex-wrap gap-2">
        <Button
          variant="lime"
          icon={<MapPinCheck className="size-4" />}
          onClick={() => setCheckIn({ open: true })}
        >
          {t("actions.checkInHere")}
        </Button>
        <Link
          to="/places/new"
          className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-brand px-5 text-[15px] font-bold text-on-brand"
        >
          <Plus className="size-4" />
          {t("actions.add")}
        </Link>
      </div>

      <Tabs
        className="mt-5"
        items={[
          { value: "mine", label: t("tabs.mine") },
          { value: "saved", label: t("tabs.saved") },
          { value: "search", label: t("tabs.search") },
        ]}
        value={tab}
        onChange={(next) => go(next)}
      />

      <div className="mt-4">
        {tab === "mine" && <MyPlaces />}
        {tab === "saved" && <SavedPlaces />}
        {tab === "search" && (
          <PlaceSearch
            initialQuery={query}
            onCheckIn={(target) => setCheckIn({ open: true, target })}
          />
        )}
      </div>

      <CheckInDialog
        open={checkIn.open}
        target={checkIn.target}
        onOpenChange={(open) => setCheckIn((current) => ({ ...current, open }))}
      />
    </Page>
  );
}

function MyPlaces() {
  const { t } = useTranslation("places");
  const { data, isPending } = useQuery(placeQueries.mine(placeClient));
  if (isPending) {
    return (
      <div className="flex flex-col gap-3">
        <Skeleton className="h-20" />
        <Skeleton className="h-20" />
        <Skeleton className="h-20" />
      </div>
    );
  }
  const places = data?.places ?? [];
  const custom = places.filter(
    (place) => place.label === geo.PlaceLabel.CUSTOM || place.label === geo.PlaceLabel.UNSPECIFIED,
  );

  return (
    <div className="flex flex-col gap-6">
      <Suggestions />

      <section aria-label={t("mine.usual")}>
        <h2 className="text-lg font-black">{t("mine.usual")}</h2>
        <p className="text-sm text-ink-soft">{t("mine.usualBody")}</p>
        <ul className="mt-3 flex flex-col gap-3">
          {([geo.PlaceLabel.HOME, geo.PlaceLabel.SCHOOL, geo.PlaceLabel.WORK] as const).map(
            (label) => {
              const place = places.find((candidate) => candidate.label === label);
              const meta = LABELS[label];
              return (
                <li key={label}>
                  {place ? (
                    <PlaceRow place={place} />
                  ) : (
                    <Link
                      to="/places/new"
                      search={{ label: meta.key }}
                      className="pressable flex items-center gap-3 rounded-card border-[2.5px] border-dashed border-line bg-surface p-3 text-ink-soft hover:border-ink-faint"
                    >
                      <span
                        className="grid size-12 shrink-0 place-items-center rounded-2xl bg-surface-2 text-2xl grayscale"
                        aria-hidden
                      >
                        {meta.emoji}
                      </span>
                      <span className="min-w-0 flex-1 font-bold">{t(`mine.add.${meta.key}`)}</span>
                      <Plus className="size-5 shrink-0" />
                    </Link>
                  )}
                </li>
              );
            },
          )}
        </ul>
      </section>

      <section aria-label={t("mine.custom")}>
        <h2 className="text-lg font-black">{t("mine.custom")}</h2>
        {custom.length === 0 ? (
          <EmptyState emoji="📍" title={t("mine.emptyTitle")} body={t("mine.emptyBody")} />
        ) : (
          <ul className="mt-3 flex flex-col gap-3">
            {custom.map((place) => (
              <li key={place.id}>
                <PlaceRow place={place} />
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}

/** "よく行く場所を登録しますか?": spots the server noticed the person keeps staying at. */
function Suggestions() {
  const { t } = useTranslation("places");
  const queryClient = useQueryClient();
  const userId = useMe()?.user?.id ?? "";
  const [dismissed, setDismissed] = useState(() => readDismissed(userId));
  const { data } = useQuery(placeQueries.suggestions(placeClient));

  const accept = useMutation({
    mutationFn: (suggestion: geo.PlaceSuggestion) =>
      placeClient().createPlace({
        name: suggestion.suggestedName || t(`labels.${labelMeta(suggestion.suggestedLabel).key}`),
        label: suggestion.suggestedLabel || geo.PlaceLabel.CUSTOM,
        center: suggestion.center,
        visibility: common.Visibility.PRIVATE,
      }),
    onSuccess: (response) => {
      toast.success(t("suggestions.accepted", { name: response.place?.name ?? "" }));
      return queryClient.invalidateQueries({ queryKey: placeKeys.all });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "places" })),
  });
  // The server keeps suggesting a spot until it is a place, so "not now" is remembered on this device.
  const dismiss = (suggestion: geo.PlaceSuggestion) =>
    setDismissed(dismissSuggestion(userId, suggestion, data?.suggestions ?? []));

  const shown = shownSuggestions(userId, data?.suggestions ?? [], dismissed);
  if (shown.length === 0) return null;
  return (
    <section aria-label={t("suggestions.title")}>
      <h2 className="text-lg font-black">{t("suggestions.title")}</h2>
      <ul className="mt-3 flex flex-col gap-3">
        {shown.map((suggestion) => {
          const meta = labelMeta(suggestion.suggestedLabel);
          const name = suggestion.suggestedName || t(`labels.${meta.key}`);
          return (
            <li
              key={suggestionMark(userId, suggestion)}
              className="sticker animate-pop-in rounded-card bg-sun p-4 text-[#17122b]"
            >
              <div className="flex items-center gap-3">
                <span className="text-3xl" aria-hidden>
                  {meta.emoji}
                </span>
                <div className="min-w-0 flex-1">
                  <p className="font-black leading-snug">{t("suggestions.question", { name })}</p>
                  <p className="text-sm font-bold opacity-75">
                    {t("suggestions.stats", {
                      count: suggestion.visitCount,
                      duration: compactDuration(Number(suggestion.totalSeconds) * 1000),
                    })}
                  </p>
                </div>
              </div>
              <div className="mt-3 flex justify-end gap-2">
                <Button
                  variant="ghost"
                  size="sm"
                  className="text-[#17122b]"
                  onClick={() => dismiss(suggestion)}
                >
                  {t("suggestions.dismiss")}
                </Button>
                <Button
                  variant="secondary"
                  size="sm"
                  loading={accept.isPending && accept.variables === suggestion}
                  onClick={() => accept.mutate(suggestion)}
                >
                  {t("suggestions.accept")}
                </Button>
              </div>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function SavedPlaces() {
  const { t } = useTranslation("places");
  const query = useInfiniteQuery(placeQueries.saved(placeClient));
  if (query.isPending) return <Skeleton className="h-20" />;
  const places = query.data?.pages.flatMap((page) => page.places) ?? [];
  if (places.length === 0)
    return <EmptyState emoji="🔖" title={t("saved.emptyTitle")} body={t("saved.emptyBody")} />;
  return (
    <>
      <ul className="flex flex-col gap-3">
        {places.map((place) => (
          <li key={place.id}>
            <PlaceRow place={place} subtitle={place.address || undefined} />
          </li>
        ))}
      </ul>
      <LoadMore query={query} />
    </>
  );
}

/**
 * Where the last search is remembered, so that coming back from a place
 * page finds the results still there. It lives in the query cache, which is
 * emptied when the account signs out. (The address only carries a search
 * someone linked to: rewriting it on every keystroke would race with the
 * tap on a result.)
 */
const LAST_SEARCH_KEY = [...placeKeys.all, "last-search"] as const;

function PlaceSearch({
  initialQuery,
  onCheckIn,
}: {
  /** A search to start with (`/places?tab=search&q=…`). */ initialQuery: string;
  onCheckIn(target: CheckInTarget): void;
}) {
  const { t } = useTranslation("places");
  const queryClient = useQueryClient();
  const [text, setText] = useState(
    () => initialQuery || (queryClient.getQueryData<string>(LAST_SEARCH_KEY) ?? ""),
  );
  const [query, setQuery] = useState(text.trim());
  // The search follows the typing with a short delay.
  useEffect(() => {
    const next = text.trim();
    queryClient.setQueryData(LAST_SEARCH_KEY, next);
    if (next === query) return;
    const timer = setTimeout(() => setQuery(next), 350);
    return () => clearTimeout(timer);
  }, [text, query, queryClient]);

  // Results near the person come first, when the device says where that is.
  const here = useQuery({
    queryKey: ["geo", "device-position"],
    queryFn: currentPosition,
    staleTime: 30_000,
    retry: false,
  });
  const near: LatLng | null = here.data ?? null;
  const settled = !here.isPending;
  const search = useQuery({
    queryKey: placeKeys.search(query, near),
    queryFn: () => placeClient().searchPlaces({ query, near: near ?? undefined, limit: 20 }),
    enabled: settled && (query.length > 0 || near !== null),
    staleTime: 60_000,
  });
  const results = search.data?.results ?? [];
  const hasGeocoderHits = results.some((result) => !result.place);

  return (
    <div>
      <TextField
        type="search"
        aria-label={t("search.label")}
        placeholder={t("search.placeholder")}
        value={text}
        maxLength={100}
        onChange={(event) => setText(event.target.value)}
        trailing={<Search className="mr-2 size-5 text-ink-faint" />}
      />
      <div className="mt-4">
        {search.isFetching && results.length === 0 ? (
          <div className="flex flex-col gap-3">
            <Skeleton className="h-16" />
            <Skeleton className="h-16" />
          </div>
        ) : search.isError ? (
          <EmptyState emoji="🌀" title={errorMessage(search.error, { namespace: "places" })} />
        ) : results.length === 0 ? (
          query ? (
            <EmptyState emoji="🔍" title={t("search.emptyTitle")} body={t("search.emptyBody")} />
          ) : (
            <EmptyState emoji="🗺️" title={t("search.hintTitle")} body={t("search.hintBody")} />
          )
        ) : (
          <>
            {!query && <h2 className="mb-3 text-lg font-black">{t("search.nearby")}</h2>}
            <ul className="flex flex-col gap-3">
              {results.map((result, index) => (
                <li key={result.place?.id ?? `${result.name}:${index}`}>
                  <SearchResult result={result} onCheckIn={onCheckIn} />
                </li>
              ))}
            </ul>
            {hasGeocoderHits && <GazetteerCredit className="mt-4 text-center" />}
          </>
        )}
      </div>
    </div>
  );
}

function SearchResult({
  result,
  onCheckIn,
}: {
  result: geo.PlaceSearchResult;
  onCheckIn(target: CheckInTarget): void;
}) {
  const { t } = useTranslation("places");
  const distance = result.distanceMeters !== undefined ? formatDistance(result.distanceMeters) : "";
  if (result.place) {
    return (
      <PlaceRow
        place={result.place}
        subtitle={
          [result.address || result.place.address, distance].filter(Boolean).join(" · ") ||
          undefined
        }
        trailing={
          result.friendsVisited.length > 0 ? (
            <span
              className="flex shrink-0 -space-x-2"
              title={t("search.friendsVisited", { count: result.friendsVisited.length })}
            >
              {result.friendsVisited.slice(0, 3).map((friend) => (
                <Avatar key={friend.id} user={friend} size="xs" />
              ))}
            </span>
          ) : undefined
        }
      />
    );
  }
  // A plain spot on the map from the place-name data: not a Tokuly place (yet).
  const position = toLatLng(result.position);
  return (
    <div className="rounded-card border-2 border-line bg-surface p-3">
      <div className="flex items-center gap-3">
        <span
          className="grid size-12 shrink-0 place-items-center rounded-2xl bg-surface-2 text-2xl"
          aria-hidden
        >
          🗾
        </span>
        <div className="min-w-0 flex-1">
          <p className="truncate font-black" data-testid="hit-name">
            {result.name}
          </p>
          <p className="truncate text-sm text-ink-soft">
            {[result.address, distance].filter(Boolean).join(" · ")}
          </p>
        </div>
      </div>
      {position && (
        <div className="mt-2 flex justify-end gap-2">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => onCheckIn({ position, name: result.name })}
          >
            {t("actions.checkIn")}
          </Button>
          <Link
            to="/places/new"
            search={{ lat: position.lat, lng: position.lng, name: result.name }}
            className="pressable sticker-sm inline-flex h-9 items-center rounded-full bg-surface px-4 text-sm font-bold"
          >
            {t("search.register")}
          </Link>
        </div>
      )}
    </div>
  );
}
