//! T3 Code's WebSocket RPC contracts as Rust types, generated at build time from
//! `schema/contracts.json` (the wire encoding of `packages/contracts`). Regenerate the schema with
//! `node apps/rune/scripts/export-contract-schemas.ts`; CI fails when it is stale.
#![allow(clippy::all, unused, non_camel_case_types, irrefutable_let_patterns)]

include!(concat!(env!("OUT_DIR"), "/generated.rs"));
