/**
 * The strings of the areas every Tokuly app has (session, map, friends, chat,
 * notifications, stories, shorts, video, notes with their lists and clips, search,
 * live, profile, settings, spaces, communities, plans, places, history, footprints, the question box, quests, school, stamps, custom emoji, the wallet and the common words), and the phone app's own screens
 * (`mobile`: React Native has no per-app glob to find them), in Japanese
 * and English, as i18next resources: one namespace per file. The web adds the namespaces of its own screens next to these,
 * and loads each one when it is first needed (`./load`).
 */
import en from "./en";
import ja from "./ja";

export { type Language, normalizeLanguage, SUPPORTED_LANGUAGES } from "./language";
export { NAMESPACES, type Namespace } from "./load";
export { en, ja };
