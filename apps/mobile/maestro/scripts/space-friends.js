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

// `me` (who listens) and a `friend` who opened a friends room on the spot and made a hangout with
// `me` invited. The friend never connects to the room's audio: the room waits for its host for
// minutes, long enough for the flow.
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
const title = `e2e room ${Date.now().toString(36)}`;
call(
  "tokuly.spaces.v1.SpaceService/CreateSpace",
  { title, audience: "SPACE_AUDIENCE_FRIENDS" },
  friend.token,
);
const hangout = `e2e hangout ${Date.now().toString(36)}`;
call(
  "tokuly.spaces.v1.HangoutService/CreateHangout",
  { name: hangout, emoji: "🛋️", inviteeIds: [me.id] },
  friend.token,
);
output.me = me.handle;
output.title = title;
output.hangout = hangout;
