# T3 Code on Rune (spike)

A native desktop client for T3 Code built on [Rune](https://github.com/anomalyco/rune), a GPU-rendered Rust UI framework. Like the web and mobile apps, it's a client of a T3 server, so it runs alongside the desktop app against the same server, local or remote.

It covers the core agent loop: pairing, the project and thread sidebar, a live timeline (streaming markdown, tools, reasoning, plans), approvals and questions, a composer with a model picker, stop, and themes, including Omarchy's environment theme, which repaints live.

## Run

```sh
cd apps/rune
cargo run --release -p t3-rune                                      # connects to this machine's server
cargo run --release -p t3-rune -- 'http://host:3773/pair#token=…'   # or pairs with any server
```

On a machine running T3 Code, the first launch connects with no link. It finds the server through `~/.t3/userdata/server-runtime.json` (`$T3CODE_HOME` overrides) and mints a token with the desktop app's `t3 pair`. That's the same trust as running the command yourself. The session shows up in Settings → Connections as "T3 Code (Rune)", and revoking it there sends the client back to the pairing screen.

For a remote server, paste a link from Settings → Connections or `t3 pair`. The pairing is saved to `$XDG_CONFIG_HOME/t3-rune/credential.json`. Linux builds need the `libxkbcommon` and `wayland` dev packages (see Rune's README).

## Layout

- `crates/t3-contracts`: Rust types for all 211 WebSocket RPCs, generated at build time from `schema/contracts.json`, which `scripts/export-contract-schemas.ts` writes from `packages/contracts`. Unions accept members the contract doesn't list, as the TypeScript client's `ForwardCompatibleUnion` does.
- `crates/t3-client`: pairing, Effect RPC over WebSocket, the lenient types the UI reads, and the shell and thread reducers. Its `tests/contracts.rs` checks every command it sends against the generated contract types. It has no UI dependency.
- `crates/t3-theme`: the web's palette derivation (`apps/web/src/themePalette.ts`), checked byte-for-byte against the real TypeScript by `tests/parity.rs`.
- `crates/t3-rune`: the UI.

## Check

```sh
cargo test -p t3-client -p t3-theme
# After upstream contract changes, refresh the schema; a stale one fails CI:
node apps/rune/scripts/export-contract-schemas.ts
# Regenerate theme assets and parity fixtures from the web code (needs the repo's node_modules):
node apps/rune/scripts/export-builtin-themes.ts && node apps/rune/scripts/theme-reference.ts
node apps/rune/scripts/terminal-reference.ts   # command output parity (crates/t3-client/tests/terminal.rs)
node apps/rune/scripts/composer-reference.ts   # @ / $ trigger parity (crates/t3-client/tests/composer.rs)
node apps/rune/scripts/shelves-reference.ts    # snooze, unread and pin rules (crates/t3-client/tests/shelves.rs)
node apps/rune/scripts/keybindings-reference.ts # shortcut resolution (crates/t3-client/tests/keybindings.rs)
# Headless render from captured wire data, no window and no network:
cargo run -p t3-client --example capture -- /tmp/fixture.json      # read-only; uses the saved pairing
T3_WIRE_FIXTURE=/tmp/fixture.json cargo test -p t3-client --test contracts   # real traffic vs. the contract
T3_RUNE_FIXTURE=/tmp/fixture.json cargo run -p t3-rune -- --snap out.png --scene omarchy --size 1280x820
```

To test against a dev server, seed it with `vp run migrate-dev-db`. Never seed it with a raw copy of `statev2.sqlite`: the dev server's recovery would resume those threads' agents.
