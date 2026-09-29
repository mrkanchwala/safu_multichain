# Vendored LayerZero crates (not committed)

`registry-oapp` builds against five LayerZero Stellar crates that are NOT in this repository:
`oapp`, `oapp-macros`, `common-utils` (crate `utils`), `common-utils-macros` (crate `common-macros`), `endpoint-v2`.

- Source: LayerZero-Labs/monorepo-external, commit `3f1cf3adadca88aa7a4ee5a7ee251c8b7fefcf2f`, read through an
  isolated disposable Codespace, then flattened into `contracts-layerzero/vendor/<crate>/`.
- Their `Cargo.toml` files were rewired to that flat layout, `soroban-sdk` is pinned to `=25.1.1`, and
  `endpoint-v2` no longer inherits from a workspace.
- Excluded here on purpose: the licence terms for redistributing that source have not been checked. Do that
  before publishing the vendor directory, or replace it with a pinned dependency.
- Toolchain: Rust 1.90.0 with `wasm32v1-none`. Pin `soroban-spec`, `soroban-spec-rust`, `soroban-sdk-macros` and
  `soroban-ledger-snapshot` to 25.1.1 in the lockfile (25.3.x needs rustc 1.91).
