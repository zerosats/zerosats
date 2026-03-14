We have a codebase that uses 1.0.0-nightly.20250723 version of barretenberg

Latest version is v5 at nightly

Do a deep detailed report on what has changed from there to now and which feautres had breaking changes

I’m compiling the release history, migration notes, changelog, and code-level repo activity from 1.0.0-nightly.20250723 to the current v5 nightly, then I’ll separate overall evolution from concrete breaking changes and upgrade risks.

I’ve confirmed the current target is v5.0.0-nightly.20260312 in AztecProtocol/aztec-packages, so I’m diffing the barretenberg/bb/bb.js surface directly. First concrete breakage: the JS CLI moved to a named bb binary, with substantial package layout changes.

I’ve now grouped the real breakages into three tracks—CLI/workflow changes, JS/TS API and runtime changes, and proof/verification target changes. The migration looks lightly disruptive around v2, then much more significant from v3 onward as bb.js shifts to pluggable native/WASM backends, constructor/export shapes change, and the ClientIVC→Chonk transition lands.

Thought for 47m 53s

I compared the official v1.0.0-nightly.20250723 tag you’re on with the current v5.0.0-nightly.20260312 tag. Between those endpoints, Barretenberg also crossed the v2.0.0-nightly.20250819, v3.0.0-nightly.20251224, and v4.0.0-nightly.20260126 major nightly lines. GitHub reports the full end-to-end compare as 5,000+ commits and 5,000+ changed files, so the useful way to review this is by user-facing surfaces: the bb CLI, the @aztec/bb.js/TS API, proving/verification workflows, packaging/runtime behavior, and compatibility signals.

My read is that this is not a routine bump if your code touches anything beyond the simplest UltraHonk CLI flow. The biggest breaks are: the IVC surface changed from ClientIVC to Chonk, verifier/proof configuration moved toward verifier_target instead of the older low-level flag mix, the CLI output-format surface changed, the TS API was re-architected around generated async/sync backends and injected API instances, the npm package/runtime behavior changed materially, and the official Noir/BB compatibility story no longer cleanly maps v5

The breaking changes that matter most
1) client_ivc became chonk

In your starting tag, the CLI --scheme accepted client_ivc, avm, and ultra_honk. In v5, the CLI --scheme accepts chonk, avm, and ultra_honk, and the execution path is explicitly wired to ChonkAPI. Any wrapper, config, or script that passes --scheme client_ivc will break and needs a migration to the Cho

This is not just a rename in the CLI. In TS, the old AztecClientBackend proved via acirProveAztecClient / acirVerifyAztecClient, while the current backend uses chonkStart, chonkLoad, chonkAccumulate, chonkProve, chonkComputeVk, chonkVerify, and chonkStats. So the IVC/aggregation model changed at both the command-line and libr

2) CLI proof configuration moved from low-level knobs to --verifier_target

The old CLI exposed and documented a lower-level mix of flags such as --oracle_hash, --disable_zk, --ipa_accumulation, --recursive, --honk_recursion, --init_kzg_accumulator, and --verifier_type. The current v5 CLI adds --verifier_target with explicit targets such as evm, evm-no-zk, noir-recursive, noir-recursive-no-zk, noir-rollup, noir-rollup-no-zk, starknet, and starknet-no-zk, and maps those targets down to hash type, ZK mode, and IPA ac

The important breaking behavior is that v5 now errors if you combine --verifier_target with --oracle_hash, --disable_zk, or --ipa_accumulation. Also, the old recursive/verifier-specific knobs are absent from the current CLI source: there is no flags.recursive, honk_recursion, init_kzg_accumulator, or verifier_type symbol in the v5 CLI file. So any code that still sets those old flags should be considered broken until

3) CLI output-format compatibility changed

In your starting tag, the CLI --output_format supported bytes, fields, bytes_and_fields, and fields_msgpack. In v5, --output_format supports only binary and json, and the JSON format includes metadata such as bb_version, scheme, and verifier_target. That is a direct compatibility break for any parser or pipeline that expected field-array or fields_msgpack outputs fr

4) The TS/JS API is a real rewrite, not a cosmetic update

At v1.0.0-nightly.20250723, the TS library was still centered on a worker/WASM model: Barretenberg.new(options) spun up WASM worker infrastructure, BackendOptions were basically {threads, memory, crsPath, wasmPath, logger}, and the main exports included BarretenbergVerifier, UltraHonkBackend, and `AztecClie

In v5, the TS layer is built on generated async/sync bindings and backend abstractions. The package exports UltraHonkVerifierBackend, UltraHonkBackend, AztecClientBackend, VerifierTarget, and backend enums/types; Barretenberg.new() now prefers native backends (NativeUnixSocket async, NativeSharedMemory sync) and falls back to WASM/WASM-worker paths; and BackendOptions now include things like bbPath, napiPath, explicit backend, maxClients, and unref. That is a surfac

5) UltraHonkBackend construction and ownership changed

Old TS code constructed UltraHonkBackend like this conceptually: (acirBytecode, backendOptions, circuitOptions), and the backend lazily created its own Barretenberg instance and SRS state internally. In v5, the constructor is (acirBytecode, api). That means lifecycle ownership moved outward: your code is expected to create/manage the Barretenberg instance and pass it in. Existing constructor calls will not survive this

6) Proof option objects changed

Old UltraHonkBackendOptions only exposed booleans like keccak, keccakZK, starknet, and starknetZK. In v5, the recommended surface is verifierTarget, with targets spanning EVM, Noir-recursive, Noir-rollup, and Starknet, each with ZK/non-ZK variants. The old boolean options are still recognized as deprecated, but v5 explicitly throws if you mix them with verifierTarget. Any code that builds option objects the old way should be updated now rather than relying on the depreca

7) BarretenbergVerifier was replaced by UltraHonkVerifierBackend

Your old code could import BarretenbergVerifier and call verifyUltraHonkProof(proofData, verificationKey). In v5, the exported verifier surface is UltraHonkVerifierBackend, and the call shape is different: verifyProof() expects proof data plus the verification key in the input object, alongside the proof settings. Code that imports or instantiates BarretenbergVerifier is a direct migra

8) getSolidityVerifier() changed in a breaking way

Old UltraHonkBackend.getSolidityVerifier(vk?) could omit the verification key and auto-compute/use a Keccak VK internally. In v5, getSolidityVerifier(vk, options?) requires the VK to be passed in and routes through circuitWriteSolidityVerifier. If your code relied on the optional-parameter behavior, it

9) AztecClientBackend.prove() changed return shape

In v1, AztecClientBackend.prove() returned [proof, vk]. In v5, prove() returns [proofFields, encodedProof, vk]. If you destructure it the old way, you will silently or noisily break your proof pipeline. This is one of the most likely runtime breaks in JS in

10) There is a quieter Node runtime requirement shift

The old TS code path for ACIR base64 decoding used Buffer in Node or atob in the browser. The current UltraHonkBackend base64 decode path errors if atob is unavailable and explicitly says “Node.js 18+ or browser required.” If part of your internal infra still runs older Node in this code path, that is another

Major changes that are not always breaking, but do change how the system behaves
Native-first runtime and packaging

At your starting point, @aztec/bb.js was documented as a “Prover/verifier executable and API,” with a standalone global install flow around bb.js, and it was effectively WASM-first. In v5, the README describes it as a library that bundles native Linux/macOS builds plus single- and multi-threaded WASM, and the runtime will use native code on supported Node platforms before falling back to WASM. That changes deployment, performance, debugging, and sandbox assumptions even when your API calls sti

The npm package CLI surface also changed: the old README told users to install globally and run bb.js, while the current package exports a bb binary. So any internal docs, scripts, Dockerfiles, or developer tooling that still invoke bb.js via the npm package should be treate

The package/build layout changed too. The old package shipped cjs-entry/ and built with tsc plus a WASM build script; the current package ships build/, adds a code-generation step, adds native-asset copying, and uses tsgo in the build. If your team patches or rebuilds @aztec/bb.js internally, this is another area to budget migratio

Msgpack and multi-language bindings became first-class

The current CLI gained msgpack schema, msgpack curve_constants, and msgpack run, while the TS layer imports generated async/sync bindings and the new Rust crate is built around the msgpack protocol with both PipeBackend and FfiBackend. That is a major architectural broadening of the public surface, even if it does not break a pure CLI consume

New CLI features

The v5 CLI adds batch_verify for Chonk proofs, vk_policy for how verification keys are handled, an optimized Solidity verifier mode, storage_budget/low-memory tuning, aztec_process for processing Aztec artifacts, and avm_simulate / avm_write_vk. Those are additive features, but they also show how far the CLI surface moved away from the older, simpler July

Under-the-hood proving changes that will probably invalidate old golden artifacts

The best official signal here is the stable v1.0.0 release notes, which explicitly called out “Fiat-Shamir the vk,” “Goblin is responsible for the merge,” and “adding support for recursive verification for ZK UltraHonk.” In the current v5 TS code, a Chonk proof is assembled from megaProof plus goblinProof.mergeProof, eccvmProof, ipaProof, and translatorProof. That combination strongly suggests that proof internals, VK derivation, recursive-artifact hashing, and serialized artifact shapes changed materially across this period. I would not use byte-for-byte proof/VK equality against your July 2025 outputs as a meaningful regres

A smaller but concrete example of that drift: generateRecursiveProofArtifacts() returned an empty vkHash string in your old version, but now returns the real vkResult.hash. That is a good capability improvement, but it changes downstream assumptions if your code treated vkHash as missing or computed

Compatibility and documentation drift

The official compatibility story is not clean at v5. In your starting tag, bbup/bb-versions.json only mapped early Noir 1.0.0 betas to old 0.82/0.84-era BB versions. In the current v5 tag, that file maps through Noir beta.19, but it still tops out at 4.0.0-nightly.20260120; there is no v5 mapping in the tagged file. So if your workflow depends on bbup-style pairing between Noir and Barretenberg, you should assume manual validation/pinning is requi

There is also a docs gap inside the v5 tag itself. The tagged cpp/src/barretenberg/bb/readme.md still documents the older UltraHonk/MegaHonk flow, old Noir compatibility hints, and even references bb _mega_honk, while the actual v5 CLI source has already moved to chonk, verifier_target, vk_policy, and binary|json output. For migration work, the tagged source/API is more trustworthy than t

The repository itself is broader now too: compared with your starting tree, the current tag adds docs, crs, and a rust crate. That is not a break by itself, but it is a good proxy for how much the integration surface expanded during t

What likely still feels familiar

The good news is that the core verbs prove, write_vk, and verify are still present, and ultra_honk is still the default CLI scheme in v5. So a very simple integration that shells out to the native bb binary for plain UltraHonk proving and verifying can still look structurally similar after t

Likewise, --oracle_hash still exists in v5; it just isn’t the preferred surface anymore, because the CLI now wants --verifier_target to own the high-level choice of verification e

My practical risk assessment

In practice, I would rate this as moderate risk if you only do plain UltraHonk CLI proving/verifying with binary outputs, high risk if you have custom CLI wrappers around recursive/Solidity/Starknet/field-output flows, and very high risk if you use @aztec/bb.js, BarretenbergVerifier, UltraHonkBackend, or AztecClientBackend directly. The farther you are from “just call bb prove/write_vk/verify,” the less this looks like a version bump and the more it looks like a

Recommended migration order

Start by pinning the matching Noir/ACIR toolchain rather than upgrading BB in isolation, because the official bbup mapping no longer gives you a clean v5 pairing and the tagged README compatibility guidanc

Then grep for the old surfaces that are most likely to break:

grep -R "client_ivc\|keccakZK\|starknetZK\|BarretenbergVerifier\|AztecClientBackend\|output_format\|honk_recursion\|init_kzg_accumulator\|verifier_type\|bb.js" .

Those terms map directly to the CLI/API surfaces that moved or disappeared between your

On the CLI side, the first rewrite pass should be: replace client_ivc with chonk, stop relying on fields/fields_msgpack outputs, move custom proof-selection logic to --verifier_target, and remove uses of the older recursive/verifier flags that are no longer present. On the TS side, the first rewrite pass should be: create/manage a Barretenberg instance yourself, pass it into UltraHonkBackend and AztecClientBackend, replace BarretenbergVerifier with UltraHonkVerifierBackend, update option objects to verifierTarget, and fix any AztecClientBackend.prove() des

Finally, regenerate all witnesses, proofs, verification keys, and recursive artifacts, and rebaseline golden tests from semantic checks rather than raw artifact equality. The combination of VK Fiat-Shamir changes, Goblin/Chonk aggregation changes, output-format changes, and API changes means old byte-for-byte artifact baselines are very likely to be

Bottom line: **basic UltraHonk CLI users can probably migrate with careful flag/output cleanup, but JS/TS users and anyone on ClientIVC/AztecClientBackend should treat this as a substantial upgrad

project.**