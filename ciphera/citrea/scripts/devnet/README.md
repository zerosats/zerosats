# Devnet-only scripts

**DO NOT RUN ON TESTNET OR MAINNET.**

Every script in this directory hard-fails before signing if the connected
RPC reports a `chainId` other than `5655` (Citrea local devnet / regtest).

These scripts:

- May embed the well-known Hardhat private key as a fallback.
- May deploy fixture contracts (USDC) with the deployer holding every
  privileged role.
- May call mock endpoints (`/dev`, `NoopVerifier`).
- May rely on `localhost:12345` defaults.

They exist for local end-to-end testing only.

Production-shaped scripts live in `../` (e.g. `deploy.ts`,
`set-burn-substitutor.ts`, `renounce-ownership.ts`). Those scripts read
a `NETWORK=dev|test|main` env var and require an explicit
`PRIVATE_KEY`/`MNEMONIC` and `RPC_URL` for any non-devnet target.

## Scripts

| Script                       | Purpose                                            |
|------------------------------|----------------------------------------------------|
| `deploy-verifiers-devnet.ts` | Deploy the NoopVerifier on local devnet            |
| `deploy-usdc-devnet.ts`      | Deploy the USDC fixture                            |
| `test-deployment-devnet.ts`  | Initialize USDC v1/v2/v2.1, mint, approve rollup   |
| `send-devnet-citrea-tx.ts`   | Smoke transaction against the local devnet RPC     |

## Typical local-flow

```bash
# 1. Verifier (NoopVerifier or aggregate, your choice)
npx hardhat run scripts/devnet/deploy-verifiers-devnet.ts

# 2. (Optional) USDC fixture
npx hardhat run scripts/devnet/deploy-usdc-devnet.ts
npx hardhat run scripts/devnet/test-deployment-devnet.ts

# 3. Rollup proper (NETWORK=dev required)
NETWORK=dev VERIFIER=0x... ERC20_ADDRESS=0x... \
  npx hardhat run scripts/deploy.ts
```
