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

```
cargo test
```

Note: these tests can take a while to run on your laptop (e.g. more than 20 minutes)


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
