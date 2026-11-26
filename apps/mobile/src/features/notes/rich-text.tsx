import { notes } from "@tokuly/api";
import { parseNoteText } from "@tokuly/core/notes";
import { router } from "expo-router";
import type { ReactNode } from "react";
import { Image, Linking, Text, View } from "react-native";
import { targetOf } from "#/features/notifications/target";
import { SERVER_URL } from "#/lib/config";

/** Links are shown without their scheme and cut after this many characters. */
const LINK_TEXT_MAX = 36;
const EMOJI_SIZE = 20;

function linkText(url: string): string {
  const bare = url.replace(/^https?:\/\/(www\.)?/i, "").replace(/\/$/, "");
  return bare.length > LINK_TEXT_MAX ? `${bare.slice(0, LINK_TEXT_MAX)}…` : bare;
}

/** A link to a screen the app has opens it; anything else goes to the system browser. */
export function openLink(url: string): void {
  const target = targetOf(url);
  if (target?.type === "screen") router.push(target.href as never);
  else void Linking.openURL(url);
}

const openTag = (tag: string) => router.push(`/notes/tag/${encodeURIComponent(tag)}`);
const openProfile = (handle: string) => router.push(`/u/${handle}`);

const absolute = (url: string) => (url.startsWith("/") ? `${SERVER_URL}${url}` : url);

const LINK = "font-bold text-brand";
const STYLES: Record<number, string> = {
  [notes.TextStyle.BOLD]: "font-black",
  [notes.TextStyle.ITALIC]: "italic",
  [notes.TextStyle.STRIKE]: "line-through",
  [notes.TextStyle.SMALL]: "text-xs",
};

/**
 * A note's text as the server parsed it (`Note.body`): paragraphs, quotes,
 * code and centred blocks; bold, italic, struck and small runs, code,
 * links, mentions, hashtags and custom emoji. Effects are not animated on
 * the phone; `x2` shows twice as large. A note from before the server
 * parsed text falls back to the plain text with its tags and links.
 */
export function NoteRichText({
  note,
  numberOfLines,
}: {
  note: Pick<notes.Note, "text" | "body" | "mentions">;
  numberOfLines?: number;
}) {
  const blocks = note.body?.blocks ?? [];
  if (blocks.length === 0) return <PlainText note={note} numberOfLines={numberOfLines} />;
  return (
    <View className="gap-1.5">
      {blocks.map((block, index) => (
        <Block key={index} block={block} numberOfLines={numberOfLines} />
      ))}
    </View>
  );
}

function Block({ block, numberOfLines }: { block: notes.TextBlock; numberOfLines?: number }) {
  const content = block.block;
  switch (content.case) {
    case "paragraph":
      return <Paragraph paragraph={content.value} numberOfLines={numberOfLines} />;
    case "quote":
      return (
        <View className="gap-1 border-l-4 border-line pl-3">
          {content.value.paragraphs.map((paragraph, index) => (
            <Paragraph key={index} paragraph={paragraph} muted />
          ))}
        </View>
      );
    case "center":
      return (
        <View className="items-center gap-1">
          {content.value.paragraphs.map((paragraph, index) => (
            <Paragraph key={index} paragraph={paragraph} centered />
          ))}
        </View>
      );
    case "code":
      return (
        <View className="rounded-xl bg-surface-2 px-3 py-2">
          <Text selectable className="font-mono text-sm text-ink">
            {content.value.code}
          </Text>
        </View>
      );
    default:
      return null;
  }
}

function Paragraph({
  paragraph,
  numberOfLines,
  muted,
  centered,
}: {
  paragraph: notes.TextParagraph;
  numberOfLines?: number;
  muted?: boolean;
  centered?: boolean;
}) {
  return (
    <Text
      numberOfLines={numberOfLines}
      className={`text-base leading-6 ${muted ? "text-ink-soft" : "text-ink"} ${centered ? "text-center" : ""}`}
    >
      <Inlines nodes={paragraph.inlines} />
    </Text>
  );
}

function Inlines({ nodes, inLink }: { nodes: notes.TextInline[]; inLink?: boolean }): ReactNode {
  return nodes.map((node, index) => <InlineNode key={index} node={node} inLink={inLink} />);
}

function InlineNode({ node, inLink }: { node: notes.TextInline; inLink?: boolean }) {
  const inline = node.inline;
  switch (inline.case) {
    case "text":
      return inline.value;
    case "span":
      return (
        <Text className={STYLES[inline.value.style] ?? ""}>
          <Inlines nodes={inline.value.children} inLink={inLink} />
        </Text>
      );
    case "code":
      return <Text className="bg-surface-2 font-mono text-sm">{inline.value}</Text>;
    case "link": {
      const { url, label } = inline.value;
      if (inLink) return label.length > 0 ? <Inlines nodes={label} inLink /> : linkText(url);
      return (
        <Text accessibilityRole="link" className={LINK} onPress={() => openLink(url)}>
          {label.length > 0 ? <Inlines nodes={label} inLink /> : linkText(url)}
        </Text>
      );
    }
    case "mention": {
      const { handle, userId } = inline.value;
      // Only a handle the server found is a person the reader may know of.
      if (!userId || inLink) return `@${handle}`;
      return (
        <Text accessibilityRole="link" className={LINK} onPress={() => openProfile(handle)}>
          @{handle}
        </Text>
      );
    }
    case "hashtag": {
      const { tag, text } = inline.value;
      if (inLink) return `#${text}`;
      return (
        <Text accessibilityRole="link" className={LINK} onPress={() => openTag(tag)}>
          #{text}
        </Text>
      );
    }
    case "emoji": {
      const { name, image } = inline.value;
      if (!image?.url) return `:${name}:`;
      return (
        <Image
          accessibilityLabel={`:${name}:`}
          source={{ uri: absolute(image.url) }}
          style={{ width: EMOJI_SIZE, height: EMOJI_SIZE }}
        />
      );
    }
    case "effect": {
      const { kind, children } = inline.value;
      return (
        <Text className={kind === notes.TextEffectKind.X2 ? "text-3xl" : ""}>
          <Inlines nodes={children} inLink={inLink} />
        </Text>
      );
    }
    default:
      return null;
  }
}

/** The text with its hashtags, known mentions and links made tappable. */
function PlainText({
  note,
  numberOfLines,
}: {
  note: Pick<notes.Note, "text" | "mentions">;
  numberOfLines?: number;
}) {
  if (!note.text) return null;
  const mentioned = new Set(note.mentions.map((mention) => mention.handle));
  return (
    <Text numberOfLines={numberOfLines} className="text-base leading-6 text-ink">
      {parseNoteText(note.text).map((segment, index) => {
        switch (segment.type) {
          case "hashtag":
            return (
              <Text key={index} className={LINK} onPress={() => openTag(segment.tag)}>
                {segment.text}
              </Text>
            );
          case "mention":
            return mentioned.has(segment.handle) ? (
              <Text key={index} className={LINK} onPress={() => openProfile(segment.handle)}>
                {segment.text}
              </Text>
            ) : (
              segment.text
            );
          case "url":
            return (
              <Text key={index} className={LINK} onPress={() => openLink(segment.url)}>
                {linkText(segment.url)}
              </Text>
            );
          default:
            return segment.text;
        }
      })}
    </Text>
  );
}
