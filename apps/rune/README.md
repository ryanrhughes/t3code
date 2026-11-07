# T3 Code on Rune (spike)

A native desktop client for T3 Code built on [Rune](https://github.com/anomalyco/rune), a GPU-rendered Rust UI framework. Like the web and mobile apps, it's a client of a T3 server, so it runs alongside the desktop app against the same server, local or remote.

It covers the core agent loop: pairing, the project and thread sidebar, a live timeline (streaming markdown, tools, reasoning, plans), approvals and questions, a composer with a model picker, stop, and themes, including Omarchy's environment theme, which repaints live.

## Run

```sh
cd apps/rune
cargo run --release -p t3-rune -- 'http://host:3773/pair#token=…'   # first run pairs
cargo run --release -p t3-rune                                      # later runs reuse the pairing
```

The pairing is saved to `$XDG_CONFIG_HOME/t3-rune/credential.json`. Get a link from Settings → Connections in the desktop app, or from `t3 pair`. Linux builds need `libxkbcommon` and `wayland` dev packages (see Rune's README).

## Layout

- `crates/t3-client`: pairing, Effect RPC over WebSocket, the contract types the client reads, and the shell and thread reducers. It has no UI dependency.
- `crates/t3-theme`: the web's palette derivation (`apps/web/src/themePalette.ts`), checked byte-for-byte against the real TypeScript by `tests/parity.rs`.
- `crates/t3-rune`: the UI.

## Check

```sh
cargo test -p t3-client -p t3-theme
# Regenerate theme assets and parity fixtures from the web code (needs the repo's node_modules):
node apps/rune/scripts/export-builtin-themes.ts && node apps/rune/scripts/theme-reference.ts
# Headless render from captured wire data, no window and no network:
cargo run -p t3-client --example capture -- /tmp/fixture.json      # read-only; uses the saved pairing
T3_RUNE_FIXTURE=/tmp/fixture.json cargo run -p t3-rune -- --snap out.png --scene omarchy --size 1280x820
```

To test against a dev server, seed it with `vp run migrate-dev-db`. Never seed it with a raw copy of `statev2.sqlite`: the dev server's recovery would resume those threads' agents.
