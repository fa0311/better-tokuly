<img src="assets/icon.svg" width="64" height="64" alt="">

# Tokuly

[![CI](https://github.com/fa0311/better-tokuly/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/fa0311/better-tokuly/actions/workflows/ci.yml)
[![Stack](https://github.com/fa0311/better-tokuly/actions/workflows/stack.yml/badge.svg?branch=main)](https://github.com/fa0311/better-tokuly/actions/workflows/stack.yml)
[![End to end](https://github.com/fa0311/better-tokuly/actions/workflows/e2e.yml/badge.svg?branch=main)](https://github.com/fa0311/better-tokuly/actions/workflows/e2e.yml)

友達と位置情報共有を楽しもう! — 位置情報共有・チャット・ノート・動画・ライブ・ショートがひとつになったスーパーアプリ。
Bump（旧 Zenly）のような地図を中心に、初代 Tokuly の機能をすべて引き継いで作り直しています。

> **開発中（アルファ版）です。** 毎日大きく変わります。リリースは [Releases](https://github.com/fa0311/better-tokuly/releases) にあります。

## いま動くもの

| | 状態 |
|---|---|
| バックエンド（全ドメイン） | 動作中 |
| Web アプリ（全ドメインの画面、PWA） | 動作中 |
| モバイルアプリ（Expo） | 地図・フレンド・チャット・通知・ストーリー・ショート・動画・ノート・ライブ。ほかはアプリ内の Web 画面（[#28](https://github.com/fa0311/better-tokuly/issues/28)） |
| 配信スタジオ（Tauri） | 未着手 |

## 構成

```
proto/       API の契約（Protobuf, Connect RPC）
backend/     Rust ワークスペース。ドメインごとに 1 クレート、バイナリは 1 つ
apps/web     Web アプリ（TanStack Start）
apps/mobile  iOS / Android アプリ（Expo）
packages/    Web とモバイルで共有する API クライアント・ロジック・文言
infra/       Docker Compose で使う設定とイメージ
docs/        機能の範囲、イベント、設定、デモ
```

## 動かす

```sh
tools/init-env.sh        # .env に秘密鍵を生成
docker compose up -d     # http://localhost
```

設定は [`docs/configuration.md`](docs/configuration.md)、ボットが動き回るデモは [`docs/demo.md`](docs/demo.md)、アップデートの前には [`docs/backup.md`](docs/backup.md) でバックアップ。

開発（インフラだけ起動して、サーバーと Web を手元で動かす）:

```sh
tools/dev/infra.sh install && tools/dev/infra.sh up && eval "$(tools/dev/infra.sh env)"
(cd backend && cargo run -p tokuly-server -- all)
pnpm install && pnpm proto:gen && pnpm dev:web          # http://localhost:3000
```

規約: [`backend/README.md`](backend/README.md) · [`apps/web/README.md`](apps/web/README.md) ·
[`apps/mobile/README.md`](apps/mobile/README.md) · [`packages/README.md`](packages/README.md) ·
機能の範囲は [`docs/domains.md`](docs/domains.md)。

## ライセンス

[MIT](LICENSE)。地名・境界データの出典は [`backend/crates/geo/data/README.md`](backend/crates/geo/data/README.md)。
配布する `backend` イメージには GPL-3.0-or-later の FFmpeg が別プログラムとして入っています（[`docs/third-party.md`](docs/third-party.md)）。

---

**English.** Tokuly is a super app built around a live map of your friends (in the spirit of Bump / Zenly):
location sharing, chat, notes, video, live streaming and shorts. A ground-up rebuild of the original Tokuly,
under heavy development. Run it with `tools/init-env.sh && docker compose up -d`. MIT; the published `backend`
image also carries a GPL-3.0-or-later FFmpeg binary.
