/**
 * The Tokuly API for TypeScript clients.
 *
 * - `createApi` builds the authenticated transport and session.
 * - Each domain's generated messages and services are re-exported as a
 *   namespace: `identity.AuthService`, `geo.FriendLocation`, ...
 */

export { create, type MessageInitShape } from "@bufbuild/protobuf";
export * from "./client.js";
export * from "./errors.js";
export * as admin from "./gen/tokuly/admin/v1/admin_pb.js";
export * as ask from "./gen/tokuly/ask/v1/ask_pb.js";
export * as chat from "./gen/tokuly/chat/v1/chat_pb.js";
export * as common from "./gen/tokuly/common/v1/common_pb.js";
export * as community from "./gen/tokuly/community/v1/community_pb.js";
export * as emoji from "./gen/tokuly/emoji/v1/emoji_pb.js";
export * as engagement from "./gen/tokuly/engagement/v1/engagement_pb.js";
export * as feed from "./gen/tokuly/feed/v1/feed_pb.js";
export * as geo from "./gen/tokuly/geo/v1/geo_pb.js";
export * as identity from "./gen/tokuly/identity/v1/identity_pb.js";
export * as live from "./gen/tokuly/live/v1/live_pb.js";
export * as media from "./gen/tokuly/media/v1/media_pb.js";
export * as moments from "./gen/tokuly/moments/v1/moments_pb.js";
export * as notes from "./gen/tokuly/notes/v1/notes_pb.js";
export * as notify from "./gen/tokuly/notify/v1/notify_pb.js";
export * as plans from "./gen/tokuly/plans/v1/plans_pb.js";
export * as quest from "./gen/tokuly/quest/v1/quest_pb.js";
export * as realtime from "./gen/tokuly/realtime/v1/realtime_pb.js";
export * as school from "./gen/tokuly/school/v1/school_pb.js";
export * as search from "./gen/tokuly/search/v1/search_pb.js";
export * as shorts from "./gen/tokuly/shorts/v1/shorts_pb.js";
export * as social from "./gen/tokuly/social/v1/social_pb.js";
export * as spaces from "./gen/tokuly/spaces/v1/spaces_pb.js";
export * as stamp from "./gen/tokuly/stamp/v1/stamp_pb.js";
export * as video from "./gen/tokuly/video/v1/video_pb.js";
export * as wallet from "./gen/tokuly/wallet/v1/wallet_pb.js";
export * from "./session.js";
export * from "./time.js";
