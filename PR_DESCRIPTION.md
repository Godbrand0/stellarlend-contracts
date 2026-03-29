# Contract Boundary Documentation

## Summary

Adds `stellar-lend/contracts/ARCHITECTURE.md` to document contract boundaries between the legacy `hello-world` crate, the canonical `lending` deployment crate, and the auxiliary `amm` crate.

The note makes the deployment recommendation explicit:

- `contracts/lending` is the canonical lending deployment target
- `contracts/amm` is an optional secondary deployment for AMM features
- `contracts/hello-world` is legacy and should not be treated as the current deployment target

## Documentation Added

- `stellar-lend/contracts/ARCHITECTURE.md`
  - deployment matrix for `hello-world` vs `lending` vs `amm`
  - trust boundaries and ownership boundaries
  - admin and guardian powers
  - token transfer flow notes
  - external call and reentrancy review
  - checked-arithmetic and parameter-bound notes

## Security Notes

- `lending` is the safest canonical target in the current tree:
  - user and admin entrypoints consistently require auth
  - pause and recovery gates are enforced on high-risk paths
  - most arithmetic uses `checked_*` or `I256`
  - flash loans include a reentrancy guard and post-callback repayment check
- `amm` should remain an auxiliary deployment until further hardening:
  - its admin helper checks stored admin equality but does not call `require_auth()`
  - swap/liquidity execution helpers are still mock protocol integrations
- `hello-world` is excluded from the active workspace and should be treated as legacy/reference code rather than the canonical deployment artifact

## Test Summary

Executed from `stellar-lend/`:

```bash
cargo test
```

Summarized result:

- test run did not complete because the host ran out of disk space while compiling dependencies
- compiler failures were environmental (`no space on device` / Windows OS error 112), not contract-test assertion failures
- the attempted run targeted active workspace crates, not the legacy `hello-world` crate

## Notes

- No contract exports or WASM interfaces changed, so no contract build step was required beyond test verification
- This change is documentation-only; no Rust modules were materially changed
- Team review is recommended before merge, especially around the documented AMM auth caveat
- Re-run `cargo test` after freeing disk space on `C:`
