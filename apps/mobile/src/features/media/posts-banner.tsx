import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { useSessionStore } from "#/features/session/store";
import { discardPost, dismissPost, type Post, retryPost, usePostsOf } from "./posts";

function Row({ post }: { post: Post }) {
  const { t } = useTranslation("mobile");
  const label = t(post.kind === "story" ? "posts.story" : "posts.short");
  const line =
    post.status === "done"
      ? t(post.kind === "story" ? "posts.doneStory" : "posts.doneShort")
      : post.status === "failed" || post.status === "interrupted"
        ? (post.error ?? t("posts.interrupted"))
        : post.status === "paused"
          ? t("posts.paused")
          : post.status === "posting"
            ? t("posts.posting")
            : t("posts.uploading", { percent: Math.round(post.progress * 100) });
  const stuck = post.status === "failed" || post.status === "interrupted";
  return (
    <View testID={`post-${post.status}`} className="gap-1.5 rounded-card bg-surface px-4 py-3">
      <View className="flex-row items-center gap-2">
        <Text className="font-black text-ink">{label}</Text>
        <Text
          accessibilityRole={stuck ? "alert" : undefined}
          numberOfLines={2}
          className={`flex-1 text-sm ${stuck ? "font-bold text-danger" : "text-ink-soft"}`}
        >
          {line}
        </Text>
      </View>
      {post.status === "uploading" && (
        <View className="h-1.5 overflow-hidden rounded-full bg-surface-3">
          <View
            className="h-full bg-brand"
            style={{ width: `${Math.round(post.progress * 100)}%` }}
          />
        </View>
      )}
      <View className="flex-row justify-end gap-4">
        {stuck && (
          <Pressable
            accessibilityRole="button"
            testID="post-retry"
            onPress={() => retryPost(post.id)}
          >
            <Text className="font-black text-brand">{t("posts.retry")}</Text>
          </Pressable>
        )}
        {post.status === "done" ? (
          <Pressable
            accessibilityRole="button"
            testID="post-dismiss"
            onPress={() => dismissPost(post.id)}
          >
            <Text className="font-black text-ink-soft">{t("posts.ok")}</Text>
          </Pressable>
        ) : (
          <Pressable
            accessibilityRole="button"
            testID="post-discard"
            onPress={() => discardPost(post.id)}
          >
            <Text className="font-black text-ink-soft">{t("posts.discard")}</Text>
          </Pressable>
        )}
      </View>
    </View>
  );
}

/** The account's posts on their way: progress, a pause for another account, the server's reason for a refusal. */
export function PostsBanner({ kind }: { kind?: Post["kind"] }) {
  const sessionId = useSessionStore((s) =>
    s.state.status === "signedIn" ? s.state.sessionId : undefined,
  );
  const posts = usePostsOf(sessionId).filter((post) => !kind || post.kind === kind);
  if (posts.length === 0) return null;
  return (
    <View className="gap-2 px-3 pb-2">
      {posts.map((post) => (
        <Row key={post.id} post={post} />
      ))}
    </View>
  );
}
