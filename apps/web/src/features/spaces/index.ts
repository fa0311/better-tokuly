/**
 * What the audio rooms area (rooms with a stage, and たまり場: always-on
 * voice rooms of friend groups) shares with other features.
 *
 *   <HangoutPresence userId={friend.user.id} />   // in a friend row or a map marker: "🛋️ いつメン" / "🎙 title" while that friend is in a hangout or room, a link to it; nothing otherwise
 *   <FriendsInRoomsStrip />                        // every friend who is in a room or hangout right now, as a row of faces to drop in on; nothing while nobody is
 *   const friends = useFriendsInRooms().data;      // the same, as data (`spaces.FriendInRoom[]`)
 *
 * All three read one cached query that the area's background listener keeps
 * current; they need nothing mounted around them.
 */
export { useFriendsInRooms } from "./api";
export { FriendsInRoomsStrip, HangoutPresence, type HangoutPresenceProps } from "./presence";
