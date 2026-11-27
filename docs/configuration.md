# 設定

設定は 3 か所にあり、上にあるものほど優先されます。どれも決めていなければ既定値です。

| 置き場所 | 中身 |
|---|---|
| 環境変数（`.env`） | 秘密鍵と、どこから届くか（`TOKULY_SECRET`、パスワード、`TOKULY_PUBLIC_URL` など）。`TOKULY_*` は何よりも優先されるので、コンテナや CI での上書きにも使える |
| `config/tokuly.toml` | それ以外の設定。全項目・既定値・説明は [`tokuly.example.toml`](tokuly.example.toml)（`tokuly config example` が生成） |
| 管理画面「サーバー設定」 | 上限・保存期間・調整用の設定など、各ドメインが許したものだけ（変更履歴と監査ログ付き）。接続先・秘密鍵・安全装置・お金に関わる設定は上の 2 つでだけ決める |

- `tokuly.toml` は `[media] max_video_hours = 3` のようにドメインごとのセクションに分かれます。
  知らないキーや型の違う値があると、キー名を表示して起動を止めます。
- 環境変数か `tokuly.toml` で決めた設定は、管理画面ではロックされます。
- リクエストごとに読む設定（レート制限、メディアの上限など）はすぐ反映されます。それ以外は再起動が必要な
  プロセスが表示されます（`docker compose restart api worker`）。
- 管理画面の「設定を書き出す」で、いまの設定（秘密鍵以外）を `tokuly.toml` として保存できます。
- `TOKULY_X=` のような空の行は「設定なし」です。空に意味がある設定（`tokuly.example.toml` に
  `empty is a value` とあるもの。`TOKULY_TRUSTED_PROXIES=` など）だけは空の値として使われます。
- `docker compose run --rm api config show` で、何がどこで決まっているかを確かめられます。

設定を追加する側のルールは [`backend/settings.md`](backend/settings.md) にあります。

## `TOKULY_SECRET` を入れ替える

漏れたかもしれないとき、または定期的に。誰もサインアウトさせず、配った URL も切らずに入れ替えられます。

1. `.env` で、いまの値を `TOKULY_SECRET_PREVIOUS` に移し、`TOKULY_SECRET` に新しい値（`openssl rand -hex 32`）を入れる。
2. 再起動する（`docker compose up -d`）。これから署名・封印するものは新しい鍵で、古い鍵のもの（ログイン中のセッション、
   メールのリンク、リンク画像や配信の URL、API トークン）は検証と開封だけ受け付けます。
3. `docker compose run --rm api rekey` で、保存してある秘密（ログイン中のセッション、2 段階認証の種、サーバー設定の秘密、
   外部連携・AI・Webhook の鍵、位置情報の共有リンク、配信キーとオーバーレイ、送金先）を新しい鍵で封印し直す。
   動かしたまま実行でき、何度実行しても同じです。
4. しばらく（目安は 2 週間）待って `TOKULY_SECRET_PREVIOUS` を消し、もう一度再起動する。漏れたときは待たずに消す。

古い鍵を消すと、待つあいだに一度も使われなかった API トークン・OAuth のトークンとクライアントシークレット
（使われたものは新しい鍵で書き直されます）、メールのリンク、2 段階認証の回復コードが使えなくなります。
回復コードは作り直してもらってください。

手順 2 の時点で一度だけ変わるもの：コミュニティの匿名の名前、位置のぼかし方、出欠のチェックインコード、Web プッシュの鍵
（ブラウザはアプリを開いたときに登録し直します。`TOKULY_VAPID_PRIVATE_KEY` を決めていれば変わりません）。
進行中だったデバイスログイン・QR での引き継ぎ・OAuth の認可（数分のもの）はやり直しになります。

## LAN の IP アドレスで動かすとき

`TOKULY_PUBLIC_URL=http://192.168.1.10` のように IP アドレスでひらくと、**パスキーは使えません**
（WebAuthn はドメイン名が必要）。サーバーは起動時に警告を出し、アプリはパスキーのボタンを出しません。
パスワードと 2 段階認証はそのまま使えます。`http://localhost` はドメイン名あつかいです。

## Caddy の前に CDN を置くとき

Caddy から見た接続元がすべて CDN になります。Caddy は知らない相手の `X-Forwarded-For` を捨てるので、
そのままでは**全員が CDN のアドレスとして数えられ**、アドレスごとのレート制限と接続数の上限にすぐかかります。
CDN が公開しているアドレス範囲を 2 か所に足してください。

- Caddy: [`infra/caddy/Caddyfile`](../infra/caddy/Caddyfile) の先頭にグローバル設定
  `{ servers { trusted_proxies static <範囲…> } }` を足す。
- Tokuly: `TOKULY_TRUSTED_PROXIES` と `TOKULY_RT_TRUSTED_PROXIES` に、既定値に加えて同じ範囲。
