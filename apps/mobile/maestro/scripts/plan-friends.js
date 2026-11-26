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

// `me` and a `friend` who planned karaoke for tomorrow evening at a place on the map, with `me`
// invited.
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
const title = `e2e karaoke ${Date.now().toString(36)}`;
const tomorrow = new Date(Date.now() + 24 * 3600 * 1000);
tomorrow.setUTCHours(9, 0, 0, 0); // 18:00 in Tokyo
call(
  "tokuly.plans.v1.PlanService/CreatePlan",
  {
    title,
    emoji: "🎤",
    time: { startAt: tomorrow.toISOString(), timeZone: "Asia/Tokyo" },
    place: {
      name: "Shibuya station",
      address: "Shibuya, Tokyo",
      location: { lat: 35.658, lng: 139.7016 },
    },
    inviteeIds: [me.id],
  },
  friend.token,
);
output.me = me.handle;
output.title = title;
