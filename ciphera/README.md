# Ciphera

A privacy preserving Zcash-like blockchain. 

Here are some highlights:

 - 🚀 Fast - runs in under 3 seconds on an iPhone
 - 🪄 Tiny - UTXO proofs are under 2.8KB
 - ✅ EVM - proofs are compatible with Ethereum 

For a detailed description of the architecture, please [download whitepaper](https://polybase.github.io/zk-rollup/whitepaper.pdf).

| Module           | Path                                    | Desc                                                            |
|------------------|-----------------------------------------|-----------------------------------------------------------------|
| Citrea Contracts | [citrea](/citrea)                       | Ethereum smart contracts to verify state transitions and proofs |
| Contracts        | [pkg/prover](/pkg/prover)               | Rust interface to Ethereum smart contracts                      |
| RPC              | [pkg/rpc](/pkg/rpc-server)              | RPC common utilities shared across all RPC services             |
| Smirk            | [pkg/smirk](/pkg/smirk)                 | Sparse merkle tree                                              |
| ZK-Circuits      | [pkg/zk-circuits](/pkg/zk-circuits)     | Halo2 + KZG ZK circuits for proving UTXO, merkle and state transitions      |
| ZK-Primitives    | [pkg/zk-primitives](/pkg/zk-primitives) | ZK primitives used across multiple modules                      |


## Tests

The main entry point for e2e coverage is `../scripts/test.sh` from this directory, or `./scripts/test.sh` from the repo root.

- `./scripts/test.sh` runs the non-ignored node e2e tests first, then the ignored full-stack integration tests.
- `./scripts/test.sh --docker` is the most reproducible way to run them if local Citrea/toolchain setup is missing.
- `./scripts/test.sh burn_tx` runs a specific ignored integration test.
- `./scripts/test.sh --verbose` enables Citrea / deploy / node logs.

Note: these are not lightweight tests. Even the non-ignored e2e suite is slow and environment-heavy. In one recent run, the non-ignored phase alone took about 9 minutes 41 seconds (MBP M2 Pro), and it depends on Citrea, Hardhat, and Barretenberg/proving setup. So these are harder to run than a normal `cargo test` sanity check.

For broad unit/integration coverage outside the Citrea-backed e2e flow, `cargo test` still works, but expect the full workspace to take a while on a laptop.


## Git LFS

We use [Git LFS](https://git-lfs.com/) for storing large files (e.g. srs params).

A one-time setup needs to be done for local development:

  1. Install `git lfs` following the instructions at https://git-lfs.com/
  2. Inside the `zk-rollup` root directory, run the following commands:

  ```bash
  $ git lfs install
  $ git lfs pull
  ```

## Contributing

We appreciate your interest in contributing to our open-source project. Your contributions help improve the project for everyone.

### Reporting Bugs

If you find a bug, please report it by [opening an issue](https://github.com/zerosats/zerosats/issues). Include as much detail as possible, including steps to reproduce the issue, the environment in which it occurs, and any relevant screenshots or code snippets.

### Suggesting Enhancements

We appreciate enhancements! To suggest a feature or enhancement, please [open an issue](https://github.com/zerosats/zerosats/issues) with a detailed description of your proposal. Explain why it is needed and how it would benefit the project.

### Submitting Pull Requests

1. Fork the repository
2. Create a new branch (`git checkout -b feature/YourFeature`)
3. Make your changes
4. Commit your changes (`git commit -m 'Add some feature'`)
5. Push to the branch (`git push origin feature/YourFeature`)
6. Open a pull request

### License

By contributing, you agree that your contributions will be licensed under the same license as the project. For more details, see [LICENSE](LICENSE).
