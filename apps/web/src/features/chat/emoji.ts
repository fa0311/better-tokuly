/** The emoji offered for reactions and in the composer's picker. */

/** One tap away on every message. */
export const QUICK_REACTIONS = ["❤️", "😂", "👍", "😮", "😢", "🙏"];

export interface EmojiGroup {
  key: "smileys" | "hearts" | "hands" | "animals" | "food" | "activities" | "objects";
  /** Shown on the group's tab. */
  icon: string;
  emoji: string[];
}

const split = (list: string) => list.split(" ");

export const EMOJI_GROUPS: EmojiGroup[] = [
  {
    key: "smileys",
    icon: "😀",
    emoji: split(
      "😀 😃 😄 😁 😆 🥹 😅 😂 🤣 🥲 😊 😇 🙂 🙃 😉 😌 😍 🥰 😘 😗 😙 😚 😋 😛 😝 😜 🤪 🤨 🧐 🤓 😎 🥸 🤩 🥳 😏 😒 😞 😔 😟 😕 🙁 😣 😖 😫 😩 🥺 😢 😭 😤 😠 😡 🤬 🤯 😳 🥵 🥶 😱 😨 😰 😥 😓 🤗 🤔 🫣 🤭 🫢 🫡 🤫 🫠 🤥 😶 😐 😑 😬 🙄 😯 😦 😧 😮 😲 🥱 😴 🤤 😪 😵 🤐 🥴 🤢 🤮 🤧 😷 🤒 🤕 🤑 🤠 😈 👿 👻 💀 👽 🤖 💩 😺 😸 😹 😻 😼 😽 🙀 😿 😾",
    ),
  },
  {
    key: "hearts",
    icon: "❤️",
    emoji: split("❤️ 🧡 💛 💚 💙 💜 🖤 🤍 🤎 🩷 🩵 🩶 💔 ❤️‍🔥 ❤️‍🩹 💕 💞 💓 💗 💖 💘 💝 💟 💌 💋 💯 💢 💥 💫 💦 💨 ✨ ⭐ 🌟 🔥 🎉 🎊 💤"),
  },
  {
    key: "hands",
    icon: "👍",
    emoji: split("👍 👎 👌 🤌 🤏 ✌️ 🤞 🫰 🤟 🤘 🤙 👈 👉 👆 👇 ☝️ ✋ 🤚 🖐️ 🖖 👋 🫶 👏 🙌 🫷 🫸 🤲 🤝 🙏 ✍️ 💪 🫵 👀 👁️ 🧠 🙇 🙋 🙆 🙅 🤷 🤦 💁 🧍 🏃 💃 🕺"),
  },
  {
    key: "animals",
    icon: "🐶",
    emoji: split("🐶 🐱 🐭 🐹 🐰 🦊 🐻 🐼 🐨 🐯 🦁 🐮 🐷 🐸 🐵 🙈 🙉 🙊 🐔 🐧 🐦 🐤 🦆 🦉 🐺 🐴 🦄 🐝 🦋 🐌 🐞 🐢 🐍 🐙 🦀 🐠 🐬 🐳 🦭 🐘 🦒 🦥 🌸 🌷 🌹 🌻 🌼 🍀 🌈 ☀️ 🌙 ☁️ ⛄ ⚡ ☔ 🌊"),
  },
  {
    key: "food",
    icon: "🍙",
    emoji: split("🍙 🍘 🍣 🍛 🍜 🍝 🍱 🍤 🍡 🍢 🍥 🥟 🍔 🍟 🍕 🌭 🌮 🥪 🍳 🥞 🧇 🍞 🥐 🧀 🍗 🍖 🥗 🍿 🍦 🍧 🍨 🍩 🍪 🎂 🍰 🧁 🍫 🍬 🍭 🍮 🍎 🍊 🍋 🍌 🍉 🍇 🍓 🍑 🍒 🥝 🍍 🥑 ☕ 🍵 🧋 🥤 🧃 🍺 🍻 🥂"),
  },
  {
    key: "activities",
    icon: "⚽",
    emoji: split("⚽ 🏀 🏈 ⚾ 🎾 🏐 🏓 🏸 🥊 🎳 ⛳ 🎣 🎿 🏂 🏄 🏊 🚴 🏆 🥇 🎮 🕹️ 🎲 🧩 🎯 🎤 🎧 🎸 🎹 🥁 🎬 🎨 📷 🎪 🎡 🎢 🏖️ 🏕️ 🗻 🗼 ⛩️ 🚃 🚄 🚌 🚗 🚲 ✈️ 🚀"),
  },
  {
    key: "objects",
    icon: "💡",
    emoji: split("💡 📱 💻 ⌚ 📚 📖 ✏️ 📝 📌 📎 ✂️ 🔑 🔒 🎁 🎈 🎀 🛍️ 💰 💴 💳 ⏰ ⏳ 📅 🔔 🔕 📣 💬 💭 🗯️ ❗ ❓ ‼️ ⁉️ ✅ ❌ ⭕ 🚫 ⚠️ 🆗 🆒 🆕 🈁 ㊗️ ㊙️ 🔴 🟠 🟡 🟢 🔵 🟣"),
  },
];

const RECENT_KEY = "tokuly.chat.recentEmoji";
const RECENT_MAX = 24;

/** The emoji this browser used last, most recent first. */
export function recentEmoji(): string[] {
  try {
    const stored = JSON.parse(window.localStorage.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(stored) ? stored.filter((value): value is string => typeof value === "string").slice(0, RECENT_MAX) : [];
  } catch {
    return [];
  }
}

export function rememberEmoji(emoji: string): void {
  const next = [emoji, ...recentEmoji().filter((used) => used !== emoji)].slice(0, RECENT_MAX);
  try {
    window.localStorage.setItem(RECENT_KEY, JSON.stringify(next));
  } catch {
    // Storage is full or blocked: the picker simply has no "recent" row.
  }
}
