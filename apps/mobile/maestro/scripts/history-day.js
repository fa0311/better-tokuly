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

// `me` lives where it is about noon right now (POSIX signs: Etc/GMT-9 is UTC+9), so the
// half hour of history below never straddles the account's midnight. The history is the
// last 28 minutes: eight at a café of their own, a walk of 840 m east, eight at a park of
// their own. Somewhere in Tokyo of its own: the database is shared.
const me = signUp("me");
const offset = (() => {
  const hours = 12 - new Date().getUTCHours();
  return hours > 12 ? hours - 24 : hours;
})();
const zone = offset === 0 ? "Etc/GMT" : `Etc/GMT${offset > 0 ? "-" : "+"}${Math.abs(offset)}`;
call("tokuly.identity.v1.AccountService/UpdateProfile", { timeZone: zone }, me.token);

const east = (from, meters) => ({
  lat: from.lat,
  lng: from.lng + meters / (111320 * Math.cos((from.lat * Math.PI) / 180)),
});
const first = { lat: 35.66 + Math.random() * 0.08, lng: 139.6 + Math.random() * 0.15 };
const second = east(first, 840);
const place = (name, emoji, center) =>
  call(
    "tokuly.geo.v1.PlaceService/CreatePlace",
    {
      name,
      emoji,
      label: "PLACE_LABEL_CUSTOM",
      center,
      radiusMeters: 80,
      visibility: "VISIBILITY_PRIVATE",
    },
    me.token,
  );
place("あさのカフェ", "☕", first);
place("ひるの公園", "🌳", second);

const sample = (position, at, activity) => ({
  position,
  accuracyMeters: 10,
  speedMps: activity === "ACTIVITY_WALKING" ? 1.4 : 0,
  activity,
  recordedAt: new Date(at).toISOString(),
});
const samples = [];
const start = Date.now() - 28 * 60000;
for (let index = 0; index <= 16; index++)
  samples.push(sample(first, start + index * 30000, "ACTIVITY_STILL"));
const walkStart = start + 8 * 60000;
for (let index = 1; index <= 60; index++)
  samples.push(sample(east(first, index * 14), walkStart + index * 10000, "ACTIVITY_WALKING"));
const arrived = walkStart + 10 * 60000;
for (let index = 1; index <= 16; index++)
  samples.push(sample(second, arrived + index * 30000, "ACTIVITY_STILL"));
const left = arrived + 8 * 60000;
for (let index = 1; index <= 9; index++)
  samples.push(sample(east(second, index * 14), left + index * 10000, "ACTIVITY_WALKING"));
call("tokuly.geo.v1.LocationService/ReportLocations", { samples }, me.token);

const pad = (value) => String(value).padStart(2, "0");
const local = new Date(Date.now() + offset * 3600000);
output.me = me.handle;
output.meId = me.id;
output.today = `${local.getUTCFullYear()}-${pad(local.getUTCMonth() + 1)}-${pad(local.getUTCDate())}`;
