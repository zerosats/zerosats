# RollupV1 V2 Upgrade Notes

This document captures security and operational rationale for the V2 changes in
`contracts/rollup/RollupV1.sol`. Keep inline comments short in contract code and
use this file for detailed context.

## Scope

V2 adds:
- deposit/TVL caps
- open-proving liveness escape hatch
- burn fees + fee sink
- inline timelock installation + ownership handoff
- upgrade hardening from review/self-review passes

## Security Invariants

### 1) Upgrade initialization must be owner-controlled

`initializeV2` is `onlyOwner reinitializer(2)`.

Why:
- `reinitializer(2)` only enforces single execution.
- Without `onlyOwner`, non-atomic upgrade flows can be front-run and let an
  attacker set fee sink/caps/timelock params.

### 2) Escape mode must not be open before V2 init

`isProvingOpen()` returns `false` when `openProvingDelay == 0`.

Why:
- pre-init defaults (`lastVerifiedAt=0`, `openProvingDelay=0`) would otherwise
  make `block.timestamp >= lastVerifiedAt + openProvingDelay` trivially true.

### 3) Reentrancy guard on `verifyRollup`

`verifyRollup` is `nonReentrant` and V2 init calls `__ReentrancyGuard_init()`.

Why:
- burn payout path performs token transfers before final state commit
  (`rootHash/blockHeight`).
- escape mode widens caller set (`onlyProverOrOpen`), so reentry risk becomes
  meaningful.

### 4) Cap accounting must follow real token outflow

In `verifyBurn`:
- TVL is decremented only when payout transfer succeeds.
- fee transfer attempts are non-reverting (`_tryTransferFee`).
- if fee transfer fails, fee remains in contract and TVL decrement uses `payout`
  rather than full `value`.

Why:
- settlement liveness must not depend on fee sink behavior.
- TVL should track tokens that actually left the contract.

## Single-token Policy in V2

V2 caps are global counters, not per-token accounting. Therefore V2 is strictly
single-token and `addToken` is disabled.

Why:
- allowing dynamic token registration makes `currentTvl` ambiguous across assets.
- TVL seed (`currentTvl = token.balanceOf(address(this))`) is defined only for
  the primary ERC20.

## Burn Fee Behavioral Change (Escrow Path)

`burnClaimed` substitutions settle at `value - fee` under V2.

Operational requirement:
- off-chain escrow/substitutor components must pay users net-of-fee and account
  for `computeBurnFee(value)`.
- fee is hard-capped on-chain (`MAX_BURN_FEE`) so governance cannot set
  confiscatory burn fees by mistake.

## Intentional Operational Semantics

- `setGlobalTvlCap` may set cap below `currentTvl`.
  - This intentionally freezes growth (new mints fail) while allowing burns to
    drain TVL back under cap.

- `setRoot` no longer exists.
  - State reset should use redeploy/testnet reset workflows, not owner root
    surgery.

## Testing Coverage (V2 suite)

`test/RollupV2.test.ts` includes:
- V2 behavior tests (ideas 1-9)
- review fixes (init ownership, pre-init open-proving guard, TVL-on-failed-burn,
  timelock brick-prevention)
- self-review fixes (reentrancy guard, fee-sink failure liveness)
