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

// A new player with one coin package bought through the development provider (it settles at
// once, no money involved): the history has a movement and the purchases list a purchase.
// Today's login bonus is left for the flow to claim.
const me = signUp("me");
const shop = call("tokuly.wallet.v1.WalletService/ListCoinPackages", {}, me.token);
if (!shop.packages || shop.packages.length === 0) throw new Error("the server sells no coins");
const bought = call(
  "tokuly.wallet.v1.WalletService/StartPurchase",
  { packageId: shop.packages[0].id, idempotencyKey: `maestro-${me.handle}` },
  me.token,
);
if (bought.purchase.status !== "PURCHASE_STATUS_COMPLETED")
  throw new Error(`the purchase is ${bought.purchase.status}`);
output.me = me.handle;
output.coins = String(bought.balance.coins);
