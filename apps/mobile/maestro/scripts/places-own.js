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

// `me` with a café of their own (private), somewhere in Tokyo of its own: the database is shared.
const me = signUp("me");
const name = `e2e cafe ${me.handle.slice(-6)}`;
const place = call(
  "tokuly.geo.v1.PlaceService/CreatePlace",
  {
    name,
    emoji: "☕",
    label: "PLACE_LABEL_CUSTOM",
    center: { lat: 35.66 + Math.random() * 0.08, lng: 139.6 + Math.random() * 0.15 },
    radiusMeters: 80,
    visibility: "VISIBILITY_PRIVATE",
  },
  me.token,
);

output.me = me.handle;
output.place = place.place.id;
output.name = name;
