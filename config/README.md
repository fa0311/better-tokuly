# config/

`tokuly.toml` goes here: Compose mounts this directory read-only at
`/etc/tokuly` in every backend container, and the backend reads
`/etc/tokuly/tokuly.toml` (`TOKULY_CONFIG`). Without the file the backend
runs on its defaults, the staff console and the environment.

`tools/init-env.sh` writes a first one. Every key, with its default and what
it does, is in [`tokuly.example.toml`](../tokuly.example.toml); a typo or a
value of the wrong type stops the backend with a message naming the key.
After editing, restart what the setting needs: `docker compose restart api worker`
(or `realtime` for `[realtime]`).

The staff console's 「サーバー設定」 shows where each value comes from and can
download the settings in force as a `tokuly.toml` (「設定を書き出す」).
