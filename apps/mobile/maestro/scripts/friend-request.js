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

// `me` (who the app signs in as), a `friend` who asked to be friends and shares a
// position in Shibuya, and a `stranger` to add by handle.
const me = signUp("me");
const friend = signUp("fr");
const stranger = signUp("st");
call(
  "tokuly.social.v1.FriendService/SendFriendRequest",
  { userId: me.id, source: "FRIEND_SOURCE_HANDLE" },
  friend.token,
);
call(
  "tokuly.geo.v1.LocationService/ReportLocations",
  {
    samples: [
      {
        position: { lat: 35.6595, lng: 139.7005 },
        accuracyMeters: 8,
        recordedAt: new Date().toISOString(),
        batteryPercent: 64,
        foreground: true,
      },
    ],
  },
  friend.token,
);
output.me = me.handle;
output.friend = friend.handle;
output.stranger = stranger.handle;
