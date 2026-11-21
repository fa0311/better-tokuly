<img src="assets/icon.svg" width="64" height="64" alt="">

# Tokuly

友達と位置情報共有を楽しもう! — 位置情報共有・チャット・ノート・動画・ライブ・ショートがひとつになったスーパーアプリ。
Bump（旧 Zenly）のような地図を中心に、初代 Tokuly の機能をすべて引き継いで作り直しています。

> **開発中です。** `feat/tokuly-rebuild` ブランチで毎日大きく変わります。
> コミットメッセージが "Work in progress" の領域はレビュー前のスナップショットです。

## いま動くもの / 作っているもの

| 領域 | 状態 |
|---|---|
| アカウント（パスワード・パスキー・TOTP）、プロフィール、プライバシー設定 | 実装・テスト済み |
| フレンド、ブロック、Bump、マップリアクション | 実装・テスト済み |
| メディア（アップロード、画像変換、HLS トランスコード） | 実装・テスト済み |
| スタンプ（ストア、ライブラリ、クリエイタースタジオ、審査） | 実装・テスト済み |
| コイン（複式台帳、購入、収益、出金） | 実装・テスト済み |
| 通知（受信箱、Web Push / FCM / APNs） | 実装・テスト済み |
| 管理・モデレーション、機能フラグ、お知らせ | 実装・テスト済み |
| リアルタイム接続（WebTransport / QUIC、WebSocket フォールバック） | 実装・テスト済み |
| 位置情報、チャット・通話、ノート・ストーリー、動画、コメント・リアクション、検索、フィード | 実装中 |
| ライブ配信、ショート | 設計済み・未着手 |
| Web アプリ（地図、認証、フレンド、プロフィール） | 動作中 |
| Web アプリ（通知、スタンプ、ウォレット、設定、管理画面） | 実装中 |
| モバイルアプリ（Expo）、配信スタジオ（Tauri） | 未着手 |
| Docker Compose | 構成は書いたが、一括起動は未検証 |

## 構成

```
proto/      API の契約（Protobuf）。Connect RPC / gRPC / gRPC-Web として提供
backend/    Rust ワークスペース。ドメインごとに 1 クレート、バイナリは 1 つ（api / realtime / worker / migrate）
apps/web    Web アプリ（TanStack Start, React 19, Tailwind 4, MapLibre GL）
packages/   TypeScript の API クライアントとリアルタイムクライアント
infra/      Docker Compose で使う設定（Caddy, LiveKit, 地図タイル）
docs/       ドメインごとの機能範囲（domains.md）とイベント一覧（events.md, events/）
tools/      開発用スクリプト
```

主な技術: Rust (axum, Connect RPC, sqlx) · PostgreSQL 18 + PostGIS · Valkey · NATS JetStream ·
ClickHouse · Meilisearch · RustFS (S3) · LiveKit · WebTransport · TanStack Start · MapLibre GL + PMTiles。
ドメイン間はイベント（トランザクショナル・アウトボックス）でつながり、機能を足すときは
proto・マイグレーション・クレートを 1 つずつ足します（`backend/README.md`）。

## 動かす

Docker Compose（**まだ一括起動を検証していません**）:

```sh
tools/init-env.sh        # .env に秘密鍵を生成
docker compose up -d     # http://localhost
```

開発（インフラをネイティブに起動）:

```sh
tools/dev/infra.sh install && tools/dev/infra.sh up
eval "$(tools/dev/infra.sh env)"
(cd backend && cargo run -p tokuly-server -- all)      # API + リアルタイム + ワーカー
pnpm install && pnpm proto:gen && pnpm dev:web         # http://localhost:3000
```

バックエンドの規約は [`backend/README.md`](backend/README.md)、Web の規約は
[`apps/web/README.md`](apps/web/README.md)、機能の範囲は [`docs/domains.md`](docs/domains.md) にあります。

---

**English.** Tokuly is a super app built around a live map of your friends (in the spirit of Bump / Zenly):
location sharing, chat and calls, notes and stories, video, live streaming, shorts, stickers and more.
This repository is a ground-up rebuild of the original Tokuly and is under heavy development; see the table
above for what works today.
