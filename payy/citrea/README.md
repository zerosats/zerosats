### Payy Hardhat Project Setup for Citrea

Hardhat environment is configured for Citrea testnet according to the documentation of Citrea project. However the
regtest dev node was unable to run via Viem-Hardhat environment so it is initialized as pure Viem-based project and
split on several scripts for investigating original Payy contract system.

A subfolder `/scripts` contains deployment scripts and some test scripts running transactions on both testnet (Chain ID

5115) and local devnet (Chain ID 5655).

Core Components:

1. RollupV1 contracts with transparent proxy upgradeability
2. Zero-knowledge proof verifiers (both legacy and Honk-based)
3. Full USDC v2.1 implementation with minting capabilities
4. Development tooling for local testing
5. Some shared functions

Setup uses deterministic accounts for development consistency.

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
- `test-deployment-devnet.ts` - Further USDC setup with initialization, minting, and approval pulled from Payy
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
