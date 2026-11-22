/** The emoji offered for reactions, grouped the way people look for them. Every entry is one emoji. */
export interface EmojiGroup {
  /** Key of the group's name in the `engagement` strings (`emoji.groups.<id>`). */
  id: "smileys" | "hearts" | "hands" | "animals" | "food" | "fun" | "symbols";
  /** The emoji shown on the group's tab. */
  icon: string;
  emoji: string[];
}

const list = (text: string): string[] => text.split(" ").filter(Boolean);

export const EMOJI_GROUPS: EmojiGroup[] = [
  {
    id: "smileys",
    icon: "😀",
    emoji: list(
      "😀 😃 😄 😁 😆 🥹 😅 😂 🤣 🥲 😊 😇 🙂 🙃 😉 😌 😍 🥰 😘 😋 😛 😜 🤪 😝 🤑 🤗 🤭 🫢 🫣 🤫 🤔 🫡 🤐 🤨 😐 😑 😶 🫥 😏 😒 🙄 😬 😮‍💨 🤥 😔 😪 🤤 😴 😷 🤒 🤕 🤢 🤮 🥵 🥶 🥴 😵 😵‍💫 🤯 🤠 🥳 🥸 😎 🤓 🧐 😕 🫤 😟 🙁 😮 😯 😲 😳 🥺 😦 😧 😨 😰 😥 😢 😭 😱 😖 😣 😞 😓 😩 😫 🥱 😤 😡 😠 🤬 😈 👿 💀 👻 👽 🤖 💩 🙈 🙉 🙊",
    ),
  },
  {
    id: "hearts",
    icon: "❤️",
    emoji: list(
      "❤️ 🩷 🧡 💛 💚 🩵 💙 💜 🖤 🩶 🤍 🤎 💔 ❤️‍🔥 ❤️‍🩹 ❣️ 💕 💞 💓 💗 💖 💘 💝 💟 💌 💋 💯 💢 💥 💫 💦 💨 💤",
    ),
  },
  {
    id: "hands",
    icon: "👍",
    emoji: list(
      "👍 👎 👏 🙌 🫶 🤝 🙏 👌 🤌 🤏 ✌️ 🤞 🫰 🤟 🤘 🤙 👈 👉 👆 👇 ☝️ ✋ 🖐️ 👋 🤚 🫡 💪 🫵 ✊ 👊 🤛 🤜 ✍️ 💅 🤳 👀 👁️ 🧠 🫀 👑",
    ),
  },
  {
    id: "animals",
    icon: "🐱",
    emoji: list(
      "🐶 🐱 🐭 🐹 🐰 🦊 🐻 🐼 🐻‍❄️ 🐨 🐯 🦁 🐮 🐷 🐸 🐵 🐔 🐧 🐦 🐤 🦆 🦉 🦄 🐝 🦋 🐌 🐞 🐢 🐍 🐙 🦀 🐠 🐬 🐳 🦈 🐘 🦒 🦥 🦦 🐾 🌸 🌷 🌹 🌻 🌼 🍀 🌱 🌵 🌴 🍁 🍂 🌙 ⭐ 🌟 ☀️ 🌈 ☁️ ⚡ ❄️ 🔥 💧 🌊",
    ),
  },
  {
    id: "food",
    icon: "🍙",
    emoji: list(
      "🍙 🍘 🍣 🍱 🍜 🍛 🍚 🍡 🍢 🥟 🍤 🍥 🍕 🍔 🍟 🌭 🌮 🥪 🍝 🥗 🍳 🧇 🥞 🍞 🥐 🧀 🍗 🍖 🍰 🎂 🧁 🍮 🍩 🍪 🍫 🍬 🍭 🍦 🍨 🍧 🍎 🍊 🍋 🍌 🍉 🍇 🍓 🍑 🍒 🥝 🍵 ☕ 🧋 🥤 🧃 🍺 🍻 🥂",
    ),
  },
  {
    id: "fun",
    icon: "🎉",
    emoji: list(
      "🎉 🎊 🎈 🎁 🎀 🏆 🥇 🎯 🎮 🕹️ 🎲 🧩 🎨 🎬 🎤 🎧 🎵 🎶 🎸 🎹 🥁 📸 📱 💻 📚 ✏️ 📝 🎓 🏫 ⚽ 🏀 ⚾ 🎾 🏐 🏓 🏸 🏃 🚴 🏊 ⛷️ 🎿 🛹 🚃 🚗 ✈️ 🚀 🗼 🗾 ⛩️ 🏯 🎡 🎢 🎆 🎇 🏝️ ⛰️ 🏕️ 💸 💰 💎",
    ),
  },
  {
    id: "symbols",
    icon: "✨",
    emoji: list(
      "✨ ⭐ 🌟 💫 🔥 💯 ✅ ❌ ⭕ ❗ ❓ ‼️ ⁉️ 💤 💢 💬 💭 🗯️ 🔔 🔕 📣 📢 🆗 🆒 🆕 🆙 🈵 🈳 🉐 ㊗️ ㊙️ 🔞 🚫 ⚠️ ♻️ 🔴 🟠 🟡 🟢 🔵 🟣 ⚫ ⚪ 🟤 🔺 🔻 🔸 🔹 ▶️ ⏸️ ⏹️ 🔁 🔀 ➕ ➖ ✖️ ➗ ♾️ 🏳️‍🌈 🇯🇵",
    ),
  },
];

/** The reactions offered first, before anyone has favourites of their own. */
export const QUICK_EMOJI = list("👍 ❤️ 😂 😮 😢 🙏 🔥 🎉");

const RECENT_KEY = "tokuly.engagement.recentEmoji";
const RECENT_LIMIT = 16;

/** The emoji this browser used last, most recent first. */
export function recentEmoji(): string[] {
  try {
    const stored: unknown = JSON.parse(window.localStorage.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(stored)
      ? stored.filter((item): item is string => typeof item === "string").slice(0, RECENT_LIMIT)
      : [];
  } catch {
    return [];
  }
}

export function rememberEmoji(emoji: string): void {
  const next = [emoji, ...recentEmoji().filter((item) => item !== emoji)].slice(0, RECENT_LIMIT);
  try {
    window.localStorage.setItem(RECENT_KEY, JSON.stringify(next));
  } catch {
    // Private mode or a full store: the picker just forgets.
  }
}

/** The row of one-tap reactions: what this person used recently, topped up with the defaults. */
export function quickEmoji(count = 8): string[] {
  return [...new Set([...recentEmoji(), ...QUICK_EMOJI])].slice(0, count);
}
