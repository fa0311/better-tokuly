import { Tabs } from "expo-router";
import { useTranslation } from "react-i18next";
import { useTheme } from "#/lib/theme";
import { useBadges } from "#/seams/badges";
import { TABS } from "#/seams/tabs";

/** The primary destinations, one tap away: the features' tabs (`#/seams/tabs`). */
export const unstable_settings = { initialRouteName: "map" };

const badgeText = (count = 0) => (count > 0 ? (count > 99 ? "99+" : count) : undefined);

export default function TabsLayout() {
  // The titles are read below in the language of this render.
  useTranslation();
  const { colors } = useTheme();
  const badges = useBadges();
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
      {TABS.map((tab) => (
        <Tabs.Screen
          key={tab.name}
          name={tab.name}
          options={{
            title: tab.title(),
            headerShown: !tab.noHeader,
            tabBarButtonTestID: `tab-${tab.name}`,
            tabBarBadge: tab.badge ? badgeText(badges[tab.badge]) : undefined,
            tabBarIcon: ({ color, size }) => <tab.icon color={color} size={size} />,
          }}
        />
      ))}
    </Tabs>
  );
}
