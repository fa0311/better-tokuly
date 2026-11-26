<img src="assets/icon.svg" width="64" height="64" alt="">

# Tokuly

[![CI](https://github.com/fa0311/better-tokuly/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/fa0311/better-tokuly/actions/workflows/ci.yml)
[![Stack](https://github.com/fa0311/better-tokuly/actions/workflows/stack.yml/badge.svg?branch=main)](https://github.com/fa0311/better-tokuly/actions/workflows/stack.yml)
[![End to end](https://github.com/fa0311/better-tokuly/actions/workflows/e2e.yml/badge.svg?branch=main)](https://github.com/fa0311/better-tokuly/actions/workflows/e2e.yml)

友達と位置情報共有を楽しもう! — 位置情報共有・チャット・ノート・動画・ライブ・ショートがひとつになったスーパーアプリ。
Bump（旧 Zenly）のような地図を中心に、初代 Tokuly の機能をすべて引き継いで作り直しています。

> **開発中（アルファ版）です。** 毎日大きく変わります。

## いま動くもの / 作っているもの

| 領域 | 状態 |
|---|---|
| アカウント（パスワード・パスキー・TOTP・アカウント切り替え）、プロフィール・プロフ帳、プライバシー設定、鍵アカウント | 動作中 |
| 位置情報（地図、ゴーストモード、足あと、訪問履歴、トリップ、SOS）、フレンド、ブロック、Bump、マップリアクション | 動作中 |
| チャット・通話（AI アシスト: Claude / OpenAI / LM Studio などを設定から選択）、ノート・ストーリー、動画、ライブ配信、ショート | 動作中 |
| コメント・リアクション、検索、フィード、通知（受信箱、Web Push / FCM / APNs、ロック画面の表示設定） | 動作中 |
| スタンプ、カスタム絵文字、コイン（台帳・購入・収益・出金）、質問箱、ミッション、予定、学校、たまり場、コミュニティ、開発者 API | 動作中 |
| 管理・モデレーション、機能フラグ、お知らせ | 動作中 |
| Web アプリ（すべての領域の画面、PWA） | 動作中 |
| モバイルアプリ（Expo）: ログイン・アカウント切り替え、ライブマップ（ゴーストモード、バックグラウンド位置共有）、フレンド、プロフィール。チャット・通知・ストーリー・ショートはこれから（[#28](https://github.com/fa0311/better-tokuly/issues/28)） | 作業中 |
| 配信スタジオ（Tauri） | 未着手 |
| Docker Compose（amd64 / arm64） | GitHub Actions で通し検証・ブラウザテスト（モバイルは Android エミュレーターで Maestro） |

全領域がセキュリティ監査（[#44](https://github.com/fa0311/better-tokuly/issues/44)）を通っています。
プレリリース版（最新は v0.2.0-alpha.1）は [Releases](https://github.com/fa0311/better-tokuly/releases) にあります
（アルファ版なので、データの互換性は保証しません）。

## 構成

```
proto/      API の契約（Protobuf）。Connect RPC / gRPC / gRPC-Web として提供
backend/    Rust ワークスペース。ドメインごとに 1 クレート、バイナリは 1 つ（api / realtime / worker / migrate）
apps/web    Web アプリ（TanStack Start, React 19, Tailwind 4, MapLibre GL）
apps/mobile iOS / Android アプリ（Expo SDK 58, Expo Router, NativeWind, MapLibre Native）
packages/   TypeScript の API・リアルタイムクライアント、共通ロジック（@tokuly/core）と文言（@tokuly/i18n）
config/     tokuly.toml（設定ファイル）の置き場所。Compose がコンテナにマウント
infra/      Docker Compose で使う設定とイメージ（Caddy, LiveKit, 地図タイル, PostGIS 入り PostgreSQL）
docs/       ドメインごとの機能範囲（domains.md）とイベント一覧（events.md, events/）
tools/      開発用スクリプト
```

主な技術: Rust (axum, Connect RPC, sqlx) · PostgreSQL 18 + PostGIS · Valkey · NATS JetStream ·
ClickHouse · Meilisearch · RustFS (S3) · LiveKit · WebTransport · TanStack Start · MapLibre GL + PMTiles。
ドメイン間はイベント（トランザクショナル・アウトボックス）でつながり、機能を足すときは
proto・マイグレーション・クレートを 1 つずつ足します（`backend/README.md`）。

## 動かす

Docker Compose:

```sh
tools/init-env.sh        # .env に秘密鍵を生成、config/tokuly.toml のひな形も
docker compose up -d     # http://localhost
```

家のサーバーに LAN の IP アドレスでつなぐ（`TOKULY_PUBLIC_URL=http://192.168.1.10`）こともできますが、
そのときは**パスキーが使えません**。パスキー（WebAuthn）はドメイン名でひらいたときだけ動く仕組みなので、
サーバーは起動時に警告を出してパスキーなしで動き、アプリはパスキーのボタンを出しません（パスワードと
2段階認証はそのまま使えます）。パスキーを使うなら `tokuly.example.home` のようなドメイン名を割り当てて
`TOKULY_PUBLIC_URL` にしてください。`http://localhost` はドメイン名あつかいなので、手元ではパスキーも動きます。

### 設定

設定は 3 か所にあり、上にあるものほど優先されます。

| 置き場所 | 中身 |
|---|---|
| 環境変数（`.env`） | 秘密鍵と、どこから届くか（`TOKULY_SECRET`、パスワード、`TOKULY_PUBLIC_URL` など）。ここに書いた `TOKULY_*` はほかのどれよりも優先されるので、コンテナや CI での上書きにも使えます |
| `config/tokuly.toml` | それ以外の設定を宣言的に。全項目と既定値と説明は [`tokuly.example.toml`](tokuly.example.toml)（`tokuly config example` が生成） |
| 管理画面「サーバー設定」 | 上限・保存期間・調整用の設定など、各ドメインが許したものだけを管理者がブラウザから変更できます（上の 2 つが決めていない場合。変更履歴と監査ログ付き）。接続先・秘密鍵・安全装置・お金に関わる設定は `tokuly.toml` か環境変数でだけ決めます |

どれも決めていなければ既定値です。`tokuly.toml` は `[media] max_video_hours = 3` のように
ドメインごとのセクションに分かれ、知らないキーや型の違う値があるとキー名を表示して起動を止めます。
`tokuly.toml` か環境変数で決めた設定は管理画面ではロックされ、どこで決まっているかが表示されます。
レート制限やメディアの上限のようにリクエストごとに読む設定はすぐ反映され、それ以外は
「再起動で反映されるよ」と必要なプロセス（api / realtime / worker）が表示されます
（`docker compose restart api worker`）。管理画面の「設定を書き出す」で、いまの設定（秘密鍵以外）を
`tokuly.toml` として保存できるので、画面で決めた設定をあとからファイルに移せます。

以前の `.env`（全部の設定が並んでいたもの）もそのまま動きます。`TOKULY_X=` のような空の行は
「設定なし」として扱われ、ほかで決まった値が使われるときは起動時に警告が出ます。ただし
`TOKULY_TRUSTED_PROXIES=`（プロキシなし）や `TOKULY_WALLET_DAILY_BONUS_COINS=`（ボーナスなし）のように
空に意味がある設定では、これまでどおり空の値として使われます（`tokuly.example.toml` に `empty is a value` とある設定）。
`docker compose run --rm api config show` で、ファイルと環境変数から何が決まっているかを確かめられます。

アプリのイメージは GitHub Container Registry のビルド済みのもの（`edge` = 通し検証を通った最新ビルド、
またはリリース版）を取得します。amd64 と arm64（Apple Silicon、ARM サーバー）の両方を配布しています。
手元のソースからビルドする場合は `docker compose up -d --build`。

アップデートで秘密鍵が増えたとき（たとえばライブ配信のメディアサーバー用の
`TOKULY_LIVE_HOOK_SECRET` と `TOKULY_MEDIAMTX_API_SECRET`）は、もう一度 `tools/init-env.sh` を実行してください。
すでにある `.env` には足りない秘密鍵だけを末尾に追記し、ほかの行には一切さわりません
（最初から作り直すのは `tools/init-env.sh --force`）。足りないまま `docker compose up` すると、
どの変数が必要かを表示して止まります。
スタック全体の起動確認は GitHub Actions の [Stack](.github/workflows/stack.yml) ワークフローが
`tools/ci/smoke.sh`（サインアップ、メール、アップロード、地図、リアルタイム接続）で行い、
画面の操作は [End to end](.github/workflows/e2e.yml) が同じスタックの上で Playwright（Web）と Maestro（Android エミュレーター）を走らせて確かめています。
テスト用のイメージはコミットごとに 1 回だけビルドし（`ci-*` として GHCR に置き、2 週間で [CI cleanup](.github/workflows/ci-cleanup.yml) が消します）、
Playwright は複数のランナーに分けて、それぞれ自分のスタックで並列に実行します。

開発（インフラだけ Docker で、または Docker なしでネイティブに起動）:

```sh
docker compose -f compose.dev.yaml up -d --wait          # Docker がある場合
tools/dev/infra.sh install && tools/dev/infra.sh up      # ない場合
eval "$(tools/dev/infra.sh env)"
(cd backend && cargo run -p tokuly-server -- all)      # API + リアルタイム + ワーカー
pnpm install && pnpm proto:gen && pnpm dev:web         # http://localhost:3000
```

モバイルアプリは開発ビルド（Expo Go ではなく）で動かします。手順は [`apps/mobile/README.md`](apps/mobile/README.md)。

バックエンドの規約は [`backend/README.md`](backend/README.md)、Web の規約は
[`apps/web/README.md`](apps/web/README.md)、Web とモバイルで共有するロジックの規約は
[`packages/README.md`](packages/README.md)、機能の範囲は [`docs/domains.md`](docs/domains.md) にあります。

## ライセンス

[MIT](LICENSE)。同梱している地名・境界データの出典とライセンスは
[`backend/crates/geo/data/README.md`](backend/crates/geo/data/README.md) を参照してください。

配布している `backend` イメージには、動画の変換に使う FFmpeg（[wader/static-ffmpeg](https://github.com/wader/static-ffmpeg)
のビルド、x264 / x265 入り）が別プログラムとして入っています。この FFmpeg は GPL-3.0-or-later です。
Tokuly 自身のコードは MIT のままですが、イメージを再配布するときは FFmpeg 部分に GPL の条件
（ライセンス文の同梱とソースコードの入手方法の案内。ソースは上記リポジトリと
[ffmpeg.org](https://ffmpeg.org/download.html) にあります）が付いてきます。

---

**English.** Tokuly is a super app built around a live map of your friends (in the spirit of Bump / Zenly):
location sharing, chat and calls, notes and stories, video, live streaming, shorts, stickers and more.
This repository is a ground-up rebuild of the original Tokuly and is under heavy development: the backend and
the web app cover every area, the Expo mobile app covers accounts, the live map and friends so far.
Run it with `tools/init-env.sh && docker compose up -d`; secrets go in `.env`, every other setting in
`config/tokuly.toml` or the staff console. See the table above for what works today.
The code is MIT; the published `backend` image also carries a GPL-3.0-or-later FFmpeg binary (run as a
separate program), whose terms apply to that binary when the image is redistributed.
