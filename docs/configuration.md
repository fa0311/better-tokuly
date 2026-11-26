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

## LAN の IP アドレスで動かすとき

`TOKULY_PUBLIC_URL=http://192.168.1.10` のように IP アドレスでひらくと、**パスキーは使えません**
（WebAuthn はドメイン名が必要）。サーバーは起動時に警告を出し、アプリはパスキーのボタンを出しません。
パスワードと 2 段階認証はそのまま使えます。`http://localhost` はドメイン名あつかいです。
