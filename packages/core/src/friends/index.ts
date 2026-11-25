/**
 * Friends and follows logic every app shares: query keys, the follow summary
 * a profile card carries, what to re-read after a change, close friends.
 * Friend lists themselves are keyed in `@tokuly/core/map` (`mapKeys.friends`).
 */
export { FOLLOW_CARD, isCloseFriends, refreshFollows, summaryFromProfile } from "./follows";
export { circleKeys, followKeys } from "./keys";
