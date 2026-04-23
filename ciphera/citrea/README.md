# Citrea Smart Contracts

Rollup smart contracts to verify the rollup state on Citrea. A subfolder `/scripts` contains deployment scripts and 
some test scripts running transactions on both testnet (Chain ID `5115`) and local devnet (Chain ID 5655).

Core components (also including outdated Ciphera components):

1. RollupV1 contracts with transparent proxy upgradeability
2. Zero-knowledge proof verifiers (both legacy and Honk-based)
3. Full USDC v2.1 implementation with minting capabilities
4. Development tooling for local testing
5. Some shared functions

Setup uses deterministic accounts for development consistency.

## Rollup V2 Notes

Detailed security and operational rationale for the V2 rollup changes is in:

- [`docs/rollup-v2-upgrade-notes.md`](docs/rollup-v2-upgrade-notes.md)

Keep inline contract comments concise and treat that document as the source of
truth for upgrade tradeoffs/invariants.

### V2 Deploy / Upgrade Requirements

The V2 safety model is only fully active when **both** of these happen:

1. `initializeV2(...)` is called on the proxy.
2. `ProxyAdmin` ownership is transferred to the same timelock as `RollupV1.owner()`.

This repo’s expected path is a **fresh deploy**:

- `npx hardhat run ./scripts/deploy.ts`

Key env vars (optional; safe defaults are used if omitted):

- `V2_PER_MINT_CAP_WEI` (default `1000000000000000` = `0.001`)
- `V2_GLOBAL_TVL_CAP_WEI` (default `10000000000000000000` = `10`)
- `V2_OPEN_PROVING_DELAY_SECONDS` (default `604800` = `7 days`)
- `V2_BURN_FEE_WEI` (default `3000000000000`, 300 sats on 18-dec BTC wrappers)
- `V2_FEE_SINK` (default deployer)
- `V2_TIMELOCK_MIN_DELAY_SECONDS` (default `86400`)
- `V2_TIMELOCK_PROPOSERS` (csv, default deployer)
- `V2_TIMELOCK_EXECUTORS` (csv, default deployer)

## Testing

The project uses Hardhat with Mocha/Chai testing framework and blockchain-specific matchers for comprehensive smart 
contract testing.

### Test deployment

In the container, run

```bash
/citrea --dev --da-layer mock --rollup-config-path /configs/mock/sequencer_rollup_config.toml \
--sequencer /configs/mock/sequencer_config.toml --genesis-paths /genesis/mock/ > /app/citrea-node.log 2>&1 &
```

then 

```bash
npx hardhat run ./scripts/deploy.ts
```

By default, it should deploy contract with NoopVerifier on the Citrea dev node. It means a mock aggregate proof 
verifier will be enabled in the deployed rollup contract.

## Deploy to Citrea

Deploy to a live network. `MNEMONIC` must be set in `.env` file and there should be some of native tokens on the 
account. The node RPC URL is hardcoded into deploy script. 

Run first:

```bash
npx hardhat run ./scripts/deploy.ts
```

Run server:

```bash
export ETHEREUM_RPC='<same as SEPOLIA_URL>' # maybe I should have just used the same env var names for hardhat deploy
export PROVER_SECRET_KEY=<same as SEPOLIA_SECRET_KEY>
export ROLLUP_CONTRACT_ADDR=...

cargo run --release server
```


### Prenet

#### Deploy

```bash
OWNER=0x6B96F1A8D65eDe8AD688716078B3DD79f9BD7323 PROVER_ADDRESS=0x6B96F1A8D65eDe8AD688716078B3DD79f9BD7323 VALIDATORS=0x6B96F1A8D65eDe8AD688716078B3DD79f9BD7323 AMOY_URL=https://polygon-amoy.g.alchemy.com/v2/9e_9NcJQ4rvg9RCsW2l7dqdbHw0VHBCf SECRET_KEY=<SECRET_KEY> GAS_PRICE_GWEI=2 yarn deploy -- --network amoy
```

#### Upgrade

```bash
ROLLUP_PROXY_ADMIN_ADDR=0x3a7122f0711822e63aa6218f4db3a6e40f97bdcf ROLLUP_CONTRACT_ADDR=0x1e44fa332fc0060164061cfedf4d3a1346a9dc38 AMOY_URL=https://polygon-amoy.g.alchemy.com/v2/9e_9NcJQ4rvg9RCsW2l7dqdbHw0VHBCf SECRET_KEY=<SECRET_KEY> yarn upgrade-rollup -- --network amoy
```

Add `UPGRADE_DEPLOY=true` to deploy the contract (not just print the calldata).

### Testnet

#### Deploy

```bash
OWNER=0x6B96F1A8D65eDe8AD688716078B3DD79f9BD7323 PROVER_ADDRESS=0x6B96F1A8D65eDe8AD688716078B3DD79f9BD7323 VALIDATORS=0x6B96F1A8D65eDe8AD688716078B3DD79f9BD7323 POLYGON_URL=https://polygon-mainnet.g.alchemy.com/v2/UrFsshbLOrSG1_cPayD3OHHi0s066Shx SECRET_KEY=<SECRET_KEY> yarn deploy -- --network polygon
```

#### Upgrade

```bash
SECRET_KEY=... ROLLUP_CONTRACT_ADDR=0x9b5df9a65c958d2d37ee1a11c1a691a2124b98d1 ROLLUP_PROXY_ADMIN_ADDR=0x55a99a706d707d033c94ffe95838e332a9e5c220  POLYGON_URL=https://polygon-mainnet.g.alchemy.com/v2/UrFsshbLOrSG1_cPayD3OHHi0s066Shx yarn upgrade-rollup -- --network polygon
```

#### Addresses

```
// remove after migration
OLD_ROLLUP_CONTRACT_ADDR=0x24baf24128af44f03d61a3e657b1cec298ef6cdc
```

```
{
  proverAddress: '0x6B96F1A8D65eDe8AD688716078B3DD79f9BD7323',
  validators: [ '0x6B96F1A8D65eDe8AD688716078B3DD79f9BD7323' ],
  ownerAddress: '0x6B96F1A8D65eDe8AD688716078B3DD79f9BD7323',
  deployerIsProxyAdmin: true
}
AGGREGATE_VERIFIER_ADDR=0x79efebbdb0dc14d3d6a359ad82aa772bb6f7fd2f
ROLLUP_V1_IMPL_ADDR=0xb72119747056a8d0b732fe1c8b45b2d028d90c8b
ROLLUP_CONTRACT_ADDR=0x9b5df9a65c958d2d37ee1a11c1a691a2124b98d1
ROLLUP_PROXY_ADMIN_ADDR=0x2b931b2c9ea3eb2ce5afd393a7dbb5aadd92fad0
```


### Mainnet

```bash
OWNER=0x230Dfb03F078B0d5E705F4624fCC915f3126B40f PROVER_ADDRESS=0x5343B904Bf837Befb2f5A256B0CD5fbF30503D38 VALIDATORS=0x41582701CB3117680687Df80bD5a2ca971bDA964 POLYGON_URL=https://polygon-mainnet.g.alchemy.com/v2/UrFsshbLOrSG1_cPayD3OHHi0s066Shx SECRET_KEY=<secret_key> yarn deploy -- --network polygon
```


#### Addresses

```
{
  proverAddress: '0x5343B904Bf837Befb2f5A256B0CD5fbF30503D38',
  validators: [ '0x41582701CB3117680687Df80bD5a2ca971bDA964' ],
  ownerAddress: '0x230Dfb03F078B0d5E705F4624fCC915f3126B40f',
  deployerIsProxyAdmin: false
}
AGGREGATE_VERIFIER_ADDR=0x4eb939ae2d1df8a1e31bbedd9283571852415834
ROLLUP_V1_IMPL_ADDR=0xfee72fcc4de2ad2972da8fa6cc388a1117147b28
ROLLUP_CONTRACT_ADDR=0xcd92281548df923141fd9b690c7c8522e12e76e6
ROLLUP_PROXY_ADMIN_ADDR=0x2db9ce1c38d18c3356d10afe367213007e2ce2d4
```

#### Upgrade

```bash
SECRET_KEY=... ROLLUP_CONTRACT_ADDR=0x4cbb5041df8d815d752239960fba5e155ba2687e ROLLUP_PROXY_ADMIN_ADDR=0xe022130f28c4e6ddf1da5be853a185fbeb84d795  POLYGON_URL=https://polygon-mainnet.g.alchemy.com/v2/UrFsshbLOrSG1_cPayD3OHHi0s066Shx yarn upgrade-rollup -- --network polygon
```

### Upgrade Rollup contract

Using `yarn upgrade-rollup`, you can upgrade a previously deployed rollup contract to a new version.

Example without a specified network:

```bash
SECRET_KEY=... ROLLUP_CONTRACT_ADDR=<proxy_contract_addr> ROLLUP_PROXY_ADMIN_ADDR=<proxy_admin_contract_addr> yarn upgrade-rollup
```

## Security Improvements

### Block Height Validation (ENG-4064)

The `verifyRollup` function in `contracts/rollup/RollupV1.sol` now includes validation to ensure new block heights are strictly greater than the current block height. This prevents:

- **Rollback Attacks**: Malicious actors cannot submit blocks with decreasing heights
- **Replay Attacks**: Same block height cannot be reused
- **Sequencing Integrity**: Maintains proper rollup block ordering
- **State Inconsistency**: Prevents breaking dependent systems expecting monotonic height increases

The validation is implemented as:
```solidity
require(height > blockHeight, "RollupV1: New block height must be greater than current");
```

### Testing

Run the security tests with:
```bash
yarn test test/SimpleBlockHeightTest.test.ts
```

## Regenerating EVM aggregate proof verifier

To re-generate EVM proof verifier, see [pkg/contracts](/pkg/prover).





### Deployment Scripts Summary

The scripts use Viem (instead of Ethers) as suggested by Hardhat 3 for blockchain interactions and deploy various smart
contracts including rollup infrastructure, verifiers, and USDC tokens.

#### 1. **Core Rollup Deployment**

- `deploy.ts` - Deploys RollupV1 contract to Citrea Testnet using Hardhat's Viem integration. This is a basic example
  script.
- `deploy-devnet.ts` - Full rollup deployment to local devnet including proxy setup. This is working script.

#### 2. **Verifier Contract Deployment**

- `deploy-verifiers-devnet.ts` - Deploys new Honk verifiers supplied as binary files for aggregate and mint operations
- `deploy-old-verifiers-devnet.ts` - Deploys legacy verifiers (aggregate, mint, burn), for test purposes

#### 3. **USDC Token Deployment**

- `deploy-usdc-devnet.ts` - Deploys USDC contract from binary
- `test-deployment-devnet.ts` - Further USDC setup with initialization, minting, and approval pulled from Ciphera
  repository

#### 4. **Testing & Utilities**

- `send-citrea-tx.ts` - Simple transaction test for testnet
- `send-devnet-citrea-tx.ts` - Transaction testing for local devnet
- `shared.ts` - Common utilities and chain configuration

### Complete Setup Sequence

1. Deploy verifier contracts (aggregate, mint, burn)
2. Deploy USDC token contract
3. Amend address variables and deploy rollup implementation contract
4. Deploy transparent proxy with initialization
5. Initialize USDC (v1 → v2 → v2.1) via `test-deployment-devnet.ts`
