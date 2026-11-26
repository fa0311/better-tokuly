import { search } from "@tokuly/api";
import { extraText, hitExtra, isKind, kindSlug } from "@tokuly/core/search";
import { Image } from "expo-image";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { mediaUrl } from "#/lib/config";

/** Text with the runs that matched the query in the brand colour (never markup). */
function Segments({
  segments,
  fallback,
  className,
  lines,
}: {
  segments: search.TextSegment[];
  fallback: string;
  className: string;
  lines: number;
}) {
  if (!segments.length && !fallback) return null;
  return (
    <Text numberOfLines={lines} className={className}>
      {segments.length
        ? segments.map((segment, index) => (
            <Text key={index} className={segment.highlighted ? "text-brand-strong" : undefined}>
              {segment.text}
            </Text>
          ))
        : fallback}
    </Text>
  );
}

/** One result: its picture, title, what kind it is and who it is from, and a snippet. */
export function HitRow({ hit, onPress }: { hit: search.SearchHit; onPress(): void }) {
  const { t } = useTranslation("search");
  const extra = hitExtra(hit);
  const kind = isKind(hit.kind) ? t(`kinds.${kindSlug(hit.kind)}`) : "";
  const handle = extraText(extra, "handle");
  const from =
    hit.kind === search.SearchKind.USER
      ? handle && `@${handle}`
      : hit.owner
        ? hit.owner.displayName || `@${hit.owner.handle}`
        : "";
  const image = mediaUrl(hit.imageUrl);
  const person = hit.kind === search.SearchKind.USER || (!image && hit.owner);
  return (
    <Pressable
      testID={`search-hit-${hit.id}`}
      accessibilityRole="button"
      onPress={onPress}
      className="flex-row items-center gap-3 border-b border-line bg-surface px-4 py-3 active:bg-surface-2"
    >
      {person ? (
        <Avatar
          user={hit.owner ?? { id: hit.id, displayName: hit.title, handle, deleted: false }}
          size="md"
        />
      ) : image ? (
        <Image source={{ uri: image }} style={{ width: 44, height: 44, borderRadius: 12 }} />
      ) : (
        <View className="h-11 w-11 items-center justify-center rounded-xl bg-brand-soft">
          <Text className="text-lg font-black text-brand-strong">
            {hit.kind === search.SearchKind.HASHTAG ? "#" : (hit.title || "?").slice(0, 1)}
          </Text>
        </View>
      )}
      <View className="min-w-0 flex-1">
        <Segments
          segments={hit.titleSegments}
          fallback={hit.title}
          lines={1}
          className="font-black text-ink"
        />
        <Text numberOfLines={1} className="text-xs font-bold text-ink-soft">
          {[kind, from].filter(Boolean).join(" · ")}
        </Text>
        <Segments
          segments={hit.snippet}
          fallback=""
          lines={2}
          className="mt-0.5 text-sm text-ink-soft"
        />
      </View>
    </Pressable>
  );
}
