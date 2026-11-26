// Arranges state through the API (Connect RPC with JSON), the way the web's browser tests
// do. SERVER comes from the command line, PASSWORD from the flow. (Maestro scripts cannot
// import each other, so each one carries these two helpers.)
function call(method, body, token) {
  const headers = { "content-type": "application/json" };
  if (token) headers.authorization = `Bearer ${token}`;
  const response = http.post(`${SERVER}/api/${method}`, { headers, body: JSON.stringify(body) });
  if (!response.ok) throw new Error(`${method}: ${response.status} ${response.body}`);
  return json(response.body);
}

function signUp(prefix) {
  const handle = `${prefix}${Date.now().toString(36)}${Math.floor(Math.random() * 1000)}`;
  const response = call("tokuly.identity.v1.AuthService/SignUp", {
    email: `${handle}@e2e.tokuly.test`,
    password: PASSWORD,
    handle,
    displayName: `${prefix} ${handle.slice(-4)}`,
    birthdate: "2007-05-05",
    acceptTerms: true,
    client: { deviceId: `maestro-${handle}` },
  });
  return { handle, token: response.accessToken, id: response.me.user.id };
}

// `me` (who watches) and a `friend` with a channel and a friends-only stream scheduled in an
// hour. There is no encoder in the stack, so the stream stays upcoming: its page, the reminder
// and the chat (open before the start) are what the flow can reach.
const me = signUp("me");
const friend = signUp("fr");
const sent = call(
  "tokuly.social.v1.FriendService/SendFriendRequest",
  { userId: me.id, source: "FRIEND_SOURCE_HANDLE" },
  friend.token,
);
call(
  "tokuly.social.v1.FriendService/RespondFriendRequest",
  { requestId: sent.request.id, accept: true },
  me.token,
);
call(
  "tokuly.video.v1.ChannelService/CreateChannel",
  { handle: `live_${friend.handle}`.slice(0, 24), name: `${friend.handle} ch` },
  friend.token,
);
const title = `e2e live ${Date.now().toString(36)}`;
call(
  "tokuly.live.v1.LiveStreamService/CreateStream",
  {
    title,
    visibility: "VISIBILITY_FRIENDS",
    scheduledAt: new Date(Date.now() + 3600 * 1000).toISOString(),
  },
  friend.token,
);
output.me = me.handle;
output.title = title;
