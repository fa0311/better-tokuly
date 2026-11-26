# Server settings: declaring one

How a domain's setting becomes a key of `tokuly.toml`, a row of the staff
console's 「サーバー設定」 and an entry of `tokuly.example.toml`. The rules
themselves are in [`backend/README.md`](../../backend/README.md), "Declaring a
setting"; what an operator sets is in [`docs/configuration.md`](../configuration.md).

## The argument

A setting is one `#[arg]` field of the domain's `clap::Args` config struct
(`crates/identity/src/config.rs`); nothing else is written by hand
(`tokuly_core::settings`). Per argument:

- an `env = "TOKULY_<DOMAIN>_<NAME>"` and a `long` flag. The key is the
  variable without `TOKULY_` and the section prefix: `TOKULY_MEDIA_MAX_VIDEO_HOURS`
  is `[media] max_video_hours` (a variable without the prefix keeps its whole name);
- a doc comment: it is the description in the console and the example file
  (the server test `every_setting_has_a_description` fails without one);
- a value with a type: `bool` with `action = clap::ArgAction::Set` (never a
  bare flag), integers, `f64`, text, or a list with `value_delimiter = ','`;
  `value_parser = [..]` gives the console a choice;
- `hide_env_values = true` for anything secret (keys, passwords, tokens,
  URLs with credentials): the console then only writes it and shows the last
  four characters, the export leaves it out (`credentials_are_secrets` checks
  the obvious names).

## What the console may change

A setting is set in
`tokuly.toml` or the environment unless its domain lists its key in
`<YourConfig>::CONSOLE_EDITABLE` (next to the struct, in `config.rs`). The
list is an allowlist on purpose: a new setting starts out of the console, so
adding one can never open a hole in it. List limits, quotas, retention,
intervals, feature tuning and texts. Never list:

- addresses the server talks to or is reached at, and origins or proxies it
  trusts (URLs, CORS / allowed origins, trusted proxies, LiveKit, MediaMTX);
- egress and SSRF switches (`*_allow_loopback`, `*_allow_private`,
  `*_allow_insecure`, allowed host lists);
- what the server executes or writes to (executable paths, directories);
- infrastructure names (index prefixes, keyspaces), credentials of the
  infrastructure, and the identity of the apps the server is registered as
  with other services (OAuth client IDs and secrets, push keys);
- protections and money: the rate limits switch, the minimum age, fees,
  exchange rates, coin rewards, the wallet's dev provider;
- privacy floors (minimum group sizes of anonymous or aggregate features).

The server test `the_console_edits_only_what_domains_allow` names the
dangerous ones that must stay out.

## The section

What clap does not know goes on the section's `SectionSpec` in
`crates/server/src/settings.rs`:

- `.editable(YourConfig::CONSOLE_EDITABLE)`: the allowlist above;
- `.live(&["key", ..])`: the domain reads the value per use through
  `core.settings.current::<YourConfig>("section")` (see `Media::live`), so a
  console change applies at once. Everything else applies on restart and the
  console says so;
- `.managed(&[(prefix, path)])`: edited on a screen of its own, which the
  settings page links to;
- `.check(check!(YourConfig))`: the domain's `validate()`, run before a
  console change is saved, at start-up and on every reload (console values a
  section fails with are set aside, so a console mistake never stops the
  server). `validate()` builds what the domain builds from its settings,
  without the network.

## The example file

After adding or changing a setting, regenerate it from the repository root:
`cargo run -p tokuly-server -- config example > tokuly.example.toml` (the
server test `the_example_file_is_current` fails until it matches).

## Precedence and empty values

Highest first: command line and environment, `tokuly.toml`
(`TOKULY_CONFIG`), the console, the default. An environment variable that is
set but empty counts as not set. Where empty means something ("no proxies",
"bonus off"), list the key in `<YourConfig>::EMPTY_IS_A_VALUE` and pass it
to `.empty_is_value(..)` (the server test `documented_empty_values_are_kept`
checks descriptions that mention an empty value).
