import { Tabs } from "expo-router";
import { MapPin, Smile, Users } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { useIncomingRequests } from "#/features/friends/data";
import { useTheme } from "#/lib/theme";

/** The primary destinations, one tap away. */
export const unstable_settings = { initialRouteName: "map" };

export default function TabsLayout() {
  const { t } = useTranslation("mobile");
  const { colors } = useTheme();
  const waiting = useIncomingRequests().data?.requests.length ?? 0;
  return (
    <Tabs
      screenOptions={{
        tabBarActiveTintColor: colors.brand,
        tabBarInactiveTintColor: colors["ink-faint"],
        tabBarStyle: { backgroundColor: colors.surface, borderTopColor: colors.line },
        tabBarLabelStyle: { fontWeight: "800" },
        headerStyle: { backgroundColor: colors.bg },
        headerTintColor: colors.ink,
        headerShadowVisible: false,
        headerTitleStyle: { fontWeight: "900" },
        sceneStyle: { backgroundColor: colors.bg },
      }}
    >
      <Tabs.Screen
        name="map"
        options={{
          title: t("tabs.map"),
          headerShown: false,
          tabBarButtonTestID: "tab-map",
          tabBarIcon: ({ color, size }) => <MapPin color={color} size={size} />,
        }}
      />
      <Tabs.Screen
        name="friends"
        options={{
          title: t("tabs.friends"),
          tabBarButtonTestID: "tab-friends",
          tabBarBadge: waiting > 0 ? waiting : undefined,
          tabBarIcon: ({ color, size }) => <Users color={color} size={size} />,
        }}
      />
      <Tabs.Screen
        name="me"
        options={{
          title: t("tabs.me"),
          tabBarButtonTestID: "tab-me",
          tabBarIcon: ({ color, size }) => <Smile color={color} size={size} />,
        }}
      />
    </Tabs>
  );
}
