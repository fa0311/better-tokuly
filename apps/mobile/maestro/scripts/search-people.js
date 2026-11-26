// Arranges state through the API (Connect RPC with JSON). SERVER comes from the command
// line, PASSWORD from the flow. (Maestro scripts cannot import each other, so each one
// carries these two helpers.)
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
    birthdate: "2000-05-05",
    acceptTerms: true,
    client: { deviceId: `maestro-${handle}` },
  });
  return { handle, token: response.accessToken, id: response.me.user.id };
}

// `me` (who the app signs in as) and someone to find, by their handle: people are
// indexed as they sign up.
const me = signUp("me");
const other = signUp("sakura");
output.me = me.handle;
output.other = other.handle;
output.otherHit = `search-hit-${other.id}`;
