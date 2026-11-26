import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Animated, Easing, Pressable, Text, View } from "react-native";
import { errorMessage } from "#/lib/errors";
import { spaceClient } from "./data";
import { emitBurst, newBurstNonce, onBurst } from "./reactions";

/** The emoji a tap away in a room. */
export const QUICK_EMOJI = ["👏", "😂", "❤️", "🔥", "😮", "🎉"];

/** At most this many float at once; the oldest give way. */
const MAX_FLOATING = 12;

interface Floating {
  key: number;
  emoji: string;
  /** Where along the bottom it rises from, 0–1. */
  lane: number;
}

/** One emoji rising and fading out. */
function Rising({ emoji, lane, onDone }: { emoji: string; lane: number; onDone(): void }) {
  const progress = useRef(new Animated.Value(0)).current;
  // Started once: a new `onDone` from the next render must not start it over.
  const done = useRef(onDone);
  done.current = onDone;
  useEffect(() => {
    Animated.timing(progress, {
      toValue: 1,
      duration: 2200,
      easing: Easing.out(Easing.quad),
      useNativeDriver: true,
    }).start(() => done.current());
  }, [progress]);
  return (
    <Animated.Text
      style={{
        position: "absolute",
        bottom: 0,
        right: `${8 + lane * 30}%`,
        fontSize: 32,
        opacity: progress.interpolate({ inputRange: [0, 0.7, 1], outputRange: [1, 1, 0] }),
        transform: [
          { translateY: progress.interpolate({ inputRange: [0, 1], outputRange: [0, -260] }) },
        ],
      }}
    >
      {emoji}
    </Animated.Text>
  );
}

/** The bursts of everyone in the room (the person's own included), rising over the room. */
export function BurstLayer({ spaceId }: { spaceId: string }) {
  const [floating, setFloating] = useState<Floating[]>([]);
  const next = useRef(0);
  useEffect(
    () =>
      onBurst((burst) => {
        if (burst.spaceId !== spaceId) return;
        const key = next.current++;
        setFloating((list) => [
          ...list.slice(-(MAX_FLOATING - 1)),
          { key, emoji: burst.emoji, lane: Math.random() },
        ]);
      }),
    [spaceId],
  );
  return (
    <View pointerEvents="none" className="absolute right-0 bottom-28 left-0 h-72">
      {floating.map((item) => (
        <Rising
          key={item.key}
          emoji={item.emoji}
          lane={item.lane}
          onDone={() => setFloating((list) => list.filter((other) => other.key !== item.key))}
        />
      ))}
    </View>
  );
}

/** The row of emoji to send; the person's own burst rises at once. */
export function EmojiRow({ spaceId, onSent }: { spaceId: string; onSent?(): void }) {
  const { t } = useTranslation("spaces");
  const send = (emoji: string) => {
    emitBurst({ spaceId, emoji });
    onSent?.();
    spaceClient()
      .sendReaction({ spaceId, emoji, nonce: newBurstNonce() })
      .catch((error: unknown) => Alert.alert(errorMessage(error, { namespace: "spaces" })));
  };
  return (
    <View testID="spaces-emoji-row" className="flex-row justify-between px-2 pb-2">
      {QUICK_EMOJI.map((emoji) => (
        <Pressable
          key={emoji}
          testID={`spaces-emoji-${emoji}`}
          accessibilityRole="button"
          accessibilityLabel={t("room.react", { emoji })}
          onPress={() => send(emoji)}
          className="size-12 items-center justify-center rounded-full bg-surface-2 active:opacity-70"
        >
          <Text className="text-2xl">{emoji}</Text>
        </Pressable>
      ))}
    </View>
  );
}
