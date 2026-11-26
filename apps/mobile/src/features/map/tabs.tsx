import { MapPin } from "lucide-react-native";
import i18n from "#/lib/i18n";
import type { TabEntry } from "#/seams/tabs";

export default [
  { name: "map", order: 10, title: () => i18n.t("mobile:tabs.map"), icon: MapPin, noHeader: true },
] satisfies TabEntry[];
