<img src="assets/icon.svg" width="64" height="64" alt="">

# Tokuly

[![CI](https://github.com/fa0311/better-tokuly/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/fa0311/better-tokuly/actions/workflows/ci.yml)
[![Stack](https://github.com/fa0311/better-tokuly/actions/workflows/stack.yml/badge.svg?branch=main)](https://github.com/fa0311/better-tokuly/actions/workflows/stack.yml)

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
| モバイルアプリ（Expo）: ログイン・アカウント切り替え、ライブマップ（ゴーストモード、バックグラウンド位置共有）、フレンド、プロフィール | 作業中 |
| 配信スタジオ（Tauri） | 未着手 |
| Docker Compose（amd64 / arm64） | GitHub Actions で通し検証・ブラウザテスト |

全領域がセキュリティ監査（[#44](https://github.com/fa0311/better-tokuly/issues/44)）を通っています。
プレリリース版は [Releases](https://github.com/fa0311/better-tokuly/releases) にあります（アルファ版なので、
データの互換性は保証しません）。

## 構成

```
proto/      API の契約（Protobuf）。Connect RPC / gRPC / gRPC-Web として提供
backend/    Rust ワークスペース。ドメインごとに 1 クレート、バイナリは 1 つ（api / realtime / worker / migrate）
apps/web    Web アプリ（TanStack Start, React 19, Tailwind 4, MapLibre GL）
apps/mobile iOS / Android アプリ（Expo SDK 58, Expo Router, NativeWind, MapLibre Native）
packages/   TypeScript の API・リアルタイムクライアント、共通ロジック（@tokuly/core）と文言（@tokuly/i18n）
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
tools/init-env.sh        # .env に秘密鍵を生成
docker compose up -d     # http://localhost
```

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
画面の操作は [Browser tests](.github/workflows/e2e.yml) が同じスタックの上で Playwright を走らせて確かめています。

開発（インフラだけ Docker で、または Docker なしでネイティブに起動）:

```sh
docker compose -f compose.dev.yaml up -d --wait          # Docker がある場合
tools/dev/infra.sh install && tools/dev/infra.sh up      # ない場合
eval "$(tools/dev/infra.sh env)"
(cd backend && cargo run -p tokuly-server -- all)      # API + リアルタイム + ワーカー
pnpm install && pnpm proto:gen && pnpm dev:web         # http://localhost:3000
```

バックエンドの規約は [`backend/README.md`](backend/README.md)、Web の規約は
[`apps/web/README.md`](apps/web/README.md)、機能の範囲は [`docs/domains.md`](docs/domains.md) にあります。

## ライセンス

[MIT](LICENSE)。同梱している地名・境界データの出典とライセンスは
[`backend/crates/geo/data/README.md`](backend/crates/geo/data/README.md) を参照してください。

---

**English.** Tokuly is a super app built around a live map of your friends (in the spirit of Bump / Zenly):
location sharing, chat and calls, notes and stories, video, live streaming, shorts, stickers and more.
This repository is a ground-up rebuild of the original Tokuly and is under heavy development; see the table
above for what works today.
