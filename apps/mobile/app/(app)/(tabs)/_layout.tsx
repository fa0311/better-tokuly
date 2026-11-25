import { Tabs } from "expo-router";
import { Bell, MapPin, MessageCircle, Smile, Users } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { useUnread as useChatUnread } from "#/features/chat/data";
import { useIncomingRequests } from "#/features/friends/data";
import { useUnread as useInboxUnread } from "#/features/notifications/data";
import { useTheme } from "#/lib/theme";

/** The primary destinations, one tap away. */
export const unstable_settings = { initialRouteName: "map" };

export default function TabsLayout() {
  const { t } = useTranslation("mobile");
  const { colors } = useTheme();
  const waiting = useIncomingRequests().data?.requests.length ?? 0;
  const chats = useChatUnread().badge;
  const inbox = useInboxUnread().total;
  const badge = (count: number) => (count > 0 ? (count > 99 ? "99+" : count) : undefined);
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
        name="chats"
        options={{
          title: t("tabs.chat"),
          tabBarButtonTestID: "tab-chats",
          tabBarBadge: badge(chats),
          tabBarIcon: ({ color, size }) => <MessageCircle color={color} size={size} />,
        }}
      />
      <Tabs.Screen
        name="friends"
        options={{
          title: t("tabs.friends"),
          tabBarButtonTestID: "tab-friends",
          tabBarBadge: badge(waiting),
          tabBarIcon: ({ color, size }) => <Users color={color} size={size} />,
        }}
      />
      <Tabs.Screen
        name="inbox"
        options={{
          title: t("tabs.inbox"),
          tabBarButtonTestID: "tab-inbox",
          tabBarBadge: badge(inbox),
          tabBarIcon: ({ color, size }) => <Bell color={color} size={size} />,
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
