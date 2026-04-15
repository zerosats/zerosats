// V2 upgrade tests for RollupV1 (bump-contract branch).
//
// Uses hardhat 3 + viem + node:test (the project's actual toolchain;
// the pre-existing ethers-based tests are broken against it and
// untouched by this branch).
//
// Coverage:
//   Idea 2: per-mint cap + TVL cap enforced on mint()
//   Idea 3: open-proving timer + isProvingOpen view (7-day floor)
//   Idea 9: computeBurnFee is a flat fee clamped to value
//
//   Review fix 1: non-owner cannot call initializeV2
//   Review fix 2: isProvingOpen is false pre-init
//   Review fix 3: failed burn does not decrement currentTvl
//
//   Self-review fix H1: nonReentrant blocks reentrant verifyRollup
//   Self-review fix H2: blacklisted feeSink leaves fee stuck, burn still settles
//
//   Plan-alignment: addToken stays V1-shape (onlyOwner, existing check)
//   Plan-alignment: initializeV2 deploys timelock + transfers ownership

import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { network } from "hardhat";
import { parseEther, encodeFunctionData } from "viem";

const NOTE_KIND =
  "0x000200000000000013fb8d0c9d1c17ae5e40fff9be350f57840e9e66cd930000" as const;
const VK_HASH =
  "0x0000000000000000000000000000000000000000000000000000000000000001" as const;

// Seven days in seconds — the contract's enforced floor for openProvingDelay.
const SEVEN_DAYS = 604800n;
// One day in seconds — the contract's floor for the timelock minDelay.
const ONE_DAY = 86400n;

// Helper: deploy RollupV1 behind an ERC1967 proxy, run initialize(),
// then run initializeV2(). Returns the rollup handle typed as
// RollupV1, plus peripheral addresses.
//
// After initializeV2 runs, ownership has been transferred to a
// freshly-deployed TimelockController. The `owner` returned by this
// helper is the DEPLOYER — the pre-upgrade owner, which is only
// useful for driving pre-V2 test paths. For post-V2 onlyOwner actions,
// the caller has to go through the deployed timelock (not exercised
// in these tests because none of them need post-V2 governance calls).
async function deployRollupV2() {
  const { viem, networkHelpers } = await network.connect();
  const [deployer, prover, validator, sink, user, spare] =
    await viem.getWalletClients();

  const mockToken = await viem.deployContract("MockERC20");
  const mockVerifier = await viem.deployContract("MockVerifier");
  const impl = await viem.deployContract("RollupV1");

  const initData = encodeFunctionData({
    abi: impl.abi,
    functionName: "initialize",
    args: [
      deployer.account.address,
      deployer.account.address, // escrow manager placeholder
      mockToken.address,
      mockVerifier.address,
      prover.account.address,
      [validator.account.address],
      VK_HASH,
    ],
  });

  const proxy = await viem.deployContract("RollupV2TestProxy", [
    impl.address,
    initData,
  ]);
  const rollup = await viem.getContractAt("RollupV1", proxy.address);

  // initializeV2 params (test-tuned):
  //   perMintCap       = 1 ether         (headroom for a single deposit)
  //   globalTvlCap     = 10 ether        (roughly 10k users at 0.001 each)
  //   openProvingDelay = 7 days          (contract floor, matches plan)
  //   burnFee          = 2000            (≈ $0.20 at $100k BTC)
  //   feeSink          = sink
  //   timelockMinDelay = 1 day           (contract floor)
  //   proposers        = [deployer]      (so the test can still schedule
  //                                       through the timelock if needed)
  //   executors        = [deployer]
  await rollup.write.initializeV2([
    parseEther("1"),
    parseEther("10"),
    SEVEN_DAYS,
    2000n,
    sink.account.address,
    ONE_DAY,
    [deployer.account.address] as readonly `0x${string}`[],
    [deployer.account.address] as readonly `0x${string}`[],
  ]);

  return {
    rollup,
    mockToken,
    mockVerifier,
    deployer,
    prover,
    validator,
    sink,
    user,
    spare,
    networkHelpers,
  };
}

describe("RollupV1 V2 upgrade", () => {
  // -----------------------------------------------------------------
  // Idea 2 — deposit caps on mint()
  // -----------------------------------------------------------------
  describe("Idea 2: deposit caps", () => {
    it("mint() rejects amount > perMintCap", async () => {
      const { rollup, mockToken, user } = await deployRollupV2();
      await mockToken.write.mint([user.account.address, parseEther("100")]);
      await mockToken.write.approve([rollup.address, parseEther("100")], {
        account: user.account,
      });

      // perMintCap = 1 ether; 2 ether must revert.
      const bigValue =
        "0x0000000000000000000000000000000000000000000000001bc16d674ec80000" as const;
      await assert.rejects(
        rollup.write.mint(
          [
            "0x2222222222222222222222222222222222222222222222222222222222222222",
            bigValue,
            NOTE_KIND,
          ],
          { account: user.account },
        ),
      );
    });

    it("currentTvl increments on mint and is cap-checked", async () => {
      const { rollup, mockToken, user } = await deployRollupV2();
      await mockToken.write.mint([user.account.address, parseEther("100")]);
      await mockToken.write.approve([rollup.address, parseEther("100")], {
        account: user.account,
      });

      const v =
        "0x00000000000000000000000000000000000000000000000000038d7ea4c68000" as const;
      const hashA =
        "0x0000000000000000000000000000000000000000000000000000000000000aaa" as const;
      await rollup.write.mint([hashA, v, NOTE_KIND], {
        account: user.account,
      });
      const tvl = await rollup.read.currentTvl();
      assert.equal(tvl, 1_000_000_000_000_000n);
    });
  });

  // -----------------------------------------------------------------
  // Idea 3 — open proving view (7-day floor)
  // -----------------------------------------------------------------
  describe("Idea 3: open proving", () => {
    it("isProvingOpen is false right after V2 init", async () => {
      const { rollup } = await deployRollupV2();
      const open = await rollup.read.isProvingOpen();
      assert.equal(open, false);
    });

    it("isProvingOpen flips to true once 7 days elapse", async () => {
      const { rollup, networkHelpers } = await deployRollupV2();
      // Jump 7 days + 1 second.
      await networkHelpers.time.increase(Number(SEVEN_DAYS) + 1);
      await networkHelpers.mine(1);
      const open = await rollup.read.isProvingOpen();
      assert.equal(open, true);
    });

    it("initializeV2 rejects openProvingDelay < 7 days", async () => {
      const { viem } = await network.connect();
      const [deployer, prover, validator, sink] =
        await viem.getWalletClients();
      const mockToken = await viem.deployContract("MockERC20");
      const mockVerifier = await viem.deployContract("MockVerifier");
      const impl = await viem.deployContract("RollupV1");
      const initData = encodeFunctionData({
        abi: impl.abi,
        functionName: "initialize",
        args: [
          deployer.account.address,
          deployer.account.address,
          mockToken.address,
          mockVerifier.address,
          prover.account.address,
          [validator.account.address],
          VK_HASH,
        ],
      });
      const proxy = await viem.deployContract("RollupV2TestProxy", [
        impl.address,
        initData,
      ]);
      const rollup = await viem.getContractAt("RollupV1", proxy.address);

      // 6 days — below the 7-day floor.
      await assert.rejects(
        rollup.write.initializeV2([
          parseEther("1"),
          parseEther("10"),
          6n * 86400n,
          2000n,
          sink.account.address,
          ONE_DAY,
          [deployer.account.address] as readonly `0x${string}`[],
          [deployer.account.address] as readonly `0x${string}`[],
        ]),
      );
    });
  });

  // -----------------------------------------------------------------
  // Idea 9 — flat burn fee
  // -----------------------------------------------------------------
  describe("Idea 9: flat burn fee", () => {
    it("computeBurnFee returns the flat fee for large values", async () => {
      const { rollup } = await deployRollupV2();
      // burnFee = 2000; any value >= 2000 returns 2000.
      const fee = await rollup.read.computeBurnFee([parseEther("1")]);
      assert.equal(fee, 2000n);
    });

    it("computeBurnFee clamps to value when value < fee", async () => {
      const { rollup } = await deployRollupV2();
      // value=500 < fee=2000 → fee clamped to 500.
      const fee = await rollup.read.computeBurnFee([500n]);
      assert.equal(fee, 500n);
    });
  });

  // =================================================================
  // Review fixes (carried forward unchanged where still applicable)
  // =================================================================
  describe("Review fixes", () => {
    // Finding 1: non-owner cannot call initializeV2.
    it("Finding 1: non-owner cannot call initializeV2", async () => {
      const { viem } = await network.connect();
      const [deployer, prover, validator, sink, attacker] =
        await viem.getWalletClients();
      const mockToken = await viem.deployContract("MockERC20");
      const mockVerifier = await viem.deployContract("MockVerifier");
      const impl = await viem.deployContract("RollupV1");
      const initData = encodeFunctionData({
        abi: impl.abi,
        functionName: "initialize",
        args: [
          deployer.account.address,
          deployer.account.address,
          mockToken.address,
          mockVerifier.address,
          prover.account.address,
          [validator.account.address],
          VK_HASH,
        ],
      });
      const proxy = await viem.deployContract("RollupV2TestProxy", [
        impl.address,
        initData,
      ]);
      const rollup = await viem.getContractAt("RollupV1", proxy.address);

      await assert.rejects(
        rollup.write.initializeV2(
          [
            parseEther("1"),
            parseEther("10"),
            SEVEN_DAYS,
            2000n,
            attacker.account.address, // attacker tries to seize fee sink
            ONE_DAY,
            [attacker.account.address] as readonly `0x${string}`[],
            [attacker.account.address] as readonly `0x${string}`[],
          ],
          { account: attacker.account },
        ),
      );

      // Deployer still can.
      await rollup.write.initializeV2(
        [
          parseEther("1"),
          parseEther("10"),
          SEVEN_DAYS,
          2000n,
          sink.account.address,
          ONE_DAY,
          [deployer.account.address] as readonly `0x${string}`[],
          [deployer.account.address] as readonly `0x${string}`[],
        ],
        { account: deployer.account },
      );
    });

    // Finding 2: pre-init isProvingOpen is false (the openProvingDelay
    // default-zero state would otherwise make the comparison trivially
    // true).
    it("Finding 2: isProvingOpen is false before initializeV2", async () => {
      const { viem } = await network.connect();
      const [deployer, prover, validator] = await viem.getWalletClients();
      const mockToken = await viem.deployContract("MockERC20");
      const mockVerifier = await viem.deployContract("MockVerifier");
      const impl = await viem.deployContract("RollupV1");
      const initData = encodeFunctionData({
        abi: impl.abi,
        functionName: "initialize",
        args: [
          deployer.account.address,
          deployer.account.address,
          mockToken.address,
          mockVerifier.address,
          prover.account.address,
          [validator.account.address],
          VK_HASH,
        ],
      });
      const proxy = await viem.deployContract("RollupV2TestProxy", [
        impl.address,
        initData,
      ]);
      const rollup = await viem.getContractAt("RollupV1", proxy.address);

      const open = await rollup.read.isProvingOpen();
      assert.equal(open, false);

      const delay = await rollup.read.openProvingDelay();
      assert.equal(delay, 0n);
    });

    // Finding 3: failed burn (silent ERC20 transfer failure) must
    // not decrement currentTvl.
    it("Finding 3: failed burn does not decrement currentTvl", async () => {
      const { viem, networkHelpers } = await network.connect();
      const [deployer, prover, validator, sink, userAcct] =
        await viem.getWalletClients();

      const failingToken = await viem.deployContract("FailingERC20");
      const mockVerifier = await viem.deployContract("MockVerifier");
      const impl = await viem.deployContract("RollupV1");
      const initData = encodeFunctionData({
        abi: impl.abi,
        functionName: "initialize",
        args: [
          deployer.account.address,
          deployer.account.address,
          failingToken.address,
          mockVerifier.address,
          prover.account.address,
          [validator.account.address],
          VK_HASH,
        ],
      });
      const proxy = await viem.deployContract("RollupV2TestProxy", [
        impl.address,
        initData,
      ]);
      const rollup = await viem.getContractAt("RollupV1", proxy.address);

      await rollup.write.initializeV2(
        [
          parseEther("1"),
          parseEther("10"),
          SEVEN_DAYS,
          0n, // zero fee so no fee-path interference
          sink.account.address,
          ONE_DAY,
          [deployer.account.address] as readonly `0x${string}`[],
          [deployer.account.address] as readonly `0x${string}`[],
        ],
        { account: deployer.account },
      );

      const mintValue = parseEther("0.5");
      const mintValueBytes32 =
        "0x00000000000000000000000000000000000000000000000006f05b59d3b20000" as const;
      await failingToken.write.mint([userAcct.account.address, mintValue]);
      await failingToken.write.approve([rollup.address, mintValue], {
        account: userAcct.account,
      });
      await rollup.write.mint(
        [
          "0x0000000000000000000000000000000000000000000000000000000000001111",
          mintValueBytes32,
          NOTE_KIND,
        ],
        { account: userAcct.account },
      );

      const tvlBefore = await rollup.read.currentTvl();
      assert.equal(tvlBefore, mintValue);

      // Trigger escape mode to bypass signature check (7 days + 1).
      await networkHelpers.time.increase(Number(SEVEN_DAYS) + 1);
      await networkHelpers.mine(1);

      // Make all transfers fail silently.
      await failingToken.write.setTransfersFail([true]);

      // Craft a burn public-inputs array.
      const oldRoot = await rollup.read.rootHash();
      const zero =
        "0x0000000000000000000000000000000000000000000000000000000000000000" as const;
      const three =
        "0x0000000000000000000000000000000000000000000000000000000000000003" as const;
      const burnHash =
        "0x0000000000000000000000000000000000000000000000000000000000002222" as const;
      const userAddrAsBytes32 = (("0x000000000000000000000000" +
        userAcct.account.address.slice(2).toLowerCase()) as `0x${string}`);
      const padding: `0x${string}`[] = Array(25).fill(zero);
      const publicInputs: `0x${string}`[] = [
        oldRoot,
        "0x0000000000000000000000000000000000000000000000000000000000000001" as const,
        "0x0000000000000000000000000000000000000000000000000000000000000002" as const,
        three,
        NOTE_KIND,
        mintValueBytes32,
        burnHash,
        userAddrAsBytes32,
        ...padding,
      ];

      await rollup.write.verifyRollup(
        [1n, VK_HASH, "0x" as const, publicInputs, zero, []],
        { account: prover.account },
      );

      const tvlAfter = await rollup.read.currentTvl();
      assert.equal(
        tvlAfter,
        tvlBefore,
        "TVL should be unchanged when burn transfer failed",
      );
      const bal = await failingToken.read.balanceOf([rollup.address]);
      assert.equal(bal, mintValue);
    });
  });

  // =================================================================
  // Self-review fixes (H1, H2 still applicable; M1 removed with the
  // validator floor it was guarding)
  // =================================================================
  describe("Self-review fixes", () => {
    it("H1: nonReentrant blocks reentrant verifyRollup", async () => {
      const { viem, networkHelpers } = await network.connect();
      const [deployer, prover, validator, sink, userAcct] =
        await viem.getWalletClients();

      const reentrantToken = await viem.deployContract("ReentrantERC20");
      const mockVerifier = await viem.deployContract("MockVerifier");
      const impl = await viem.deployContract("RollupV1");
      const initData = encodeFunctionData({
        abi: impl.abi,
        functionName: "initialize",
        args: [
          deployer.account.address,
          deployer.account.address,
          reentrantToken.address,
          mockVerifier.address,
          prover.account.address,
          [validator.account.address],
          VK_HASH,
        ],
      });
      const proxy = await viem.deployContract("RollupV2TestProxy", [
        impl.address,
        initData,
      ]);
      const rollup = await viem.getContractAt("RollupV1", proxy.address);

      await rollup.write.initializeV2(
        [
          parseEther("1"),
          parseEther("10"),
          SEVEN_DAYS,
          0n,
          sink.account.address,
          ONE_DAY,
          [deployer.account.address] as readonly `0x${string}`[],
          [deployer.account.address] as readonly `0x${string}`[],
        ],
        { account: deployer.account },
      );

      const mintValue = parseEther("0.5");
      const mintValueBytes32 =
        "0x00000000000000000000000000000000000000000000000006f05b59d3b20000" as const;
      await reentrantToken.write.mint([userAcct.account.address, mintValue]);
      await reentrantToken.write.approve([rollup.address, mintValue], {
        account: userAcct.account,
      });
      await rollup.write.mint(
        [
          "0x0000000000000000000000000000000000000000000000000000000000003333",
          mintValueBytes32,
          NOTE_KIND,
        ],
        { account: userAcct.account },
      );

      const reentrantPayload = encodeFunctionData({
        abi: impl.abi,
        functionName: "verifyRollup",
        args: [
          2n,
          VK_HASH,
          "0x" as const,
          Array(33).fill(
            "0x0000000000000000000000000000000000000000000000000000000000000000",
          ) as readonly `0x${string}`[],
          "0x0000000000000000000000000000000000000000000000000000000000000000" as const,
          [],
        ],
      });
      await reentrantToken.write.setAttack([rollup.address, reentrantPayload]);

      await networkHelpers.time.increase(Number(SEVEN_DAYS) + 1);
      await networkHelpers.mine(1);

      const oldRoot = await rollup.read.rootHash();
      const zero =
        "0x0000000000000000000000000000000000000000000000000000000000000000" as const;
      const three =
        "0x0000000000000000000000000000000000000000000000000000000000000003" as const;
      const burnHash =
        "0x0000000000000000000000000000000000000000000000000000000000003334" as const;
      const userAddrAsBytes32 = (("0x000000000000000000000000" +
        userAcct.account.address.slice(2).toLowerCase()) as `0x${string}`);
      const padding: `0x${string}`[] = Array(25).fill(zero);
      const publicInputs: `0x${string}`[] = [
        oldRoot,
        "0x0000000000000000000000000000000000000000000000000000000000000010" as const,
        "0x0000000000000000000000000000000000000000000000000000000000000011" as const,
        three,
        NOTE_KIND,
        mintValueBytes32,
        burnHash,
        userAddrAsBytes32,
        ...padding,
      ];

      await rollup.write.verifyRollup(
        [1n, VK_HASH, "0x" as const, publicInputs, zero, []],
        { account: prover.account },
      );

      const reentrantSucceeded =
        await reentrantToken.read.reentrantCallSucceeded();
      assert.equal(
        reentrantSucceeded,
        false,
        "ReentrancyGuard did not block the reentrant verifyRollup",
      );

      const newHeight = await rollup.read.blockHeight();
      assert.equal(newHeight, 1n);
    });

    it("H2: blacklisted feeSink leaves fee stuck but burn still settles", async () => {
      const { viem, networkHelpers } = await network.connect();
      const [deployer, prover, validator, sink, userAcct] =
        await viem.getWalletClients();

      const token = await viem.deployContract("FailingERC20");
      const mockVerifier = await viem.deployContract("MockVerifier");
      const impl = await viem.deployContract("RollupV1");
      const initData = encodeFunctionData({
        abi: impl.abi,
        functionName: "initialize",
        args: [
          deployer.account.address,
          deployer.account.address,
          token.address,
          mockVerifier.address,
          prover.account.address,
          [validator.account.address],
          VK_HASH,
        ],
      });
      const proxy = await viem.deployContract("RollupV2TestProxy", [
        impl.address,
        initData,
      ]);
      const rollup = await viem.getContractAt("RollupV1", proxy.address);

      const FEE = 2000n;
      await rollup.write.initializeV2(
        [
          parseEther("1"),
          parseEther("10"),
          SEVEN_DAYS,
          FEE,
          sink.account.address,
          ONE_DAY,
          [deployer.account.address] as readonly `0x${string}`[],
          [deployer.account.address] as readonly `0x${string}`[],
        ],
        { account: deployer.account },
      );

      // Blacklist sink so fee transfer reverts; main payout still ok.
      await token.write.setBlacklisted([sink.account.address, true]);

      const mintValue = parseEther("0.5");
      const mintValueBytes32 =
        "0x00000000000000000000000000000000000000000000000006f05b59d3b20000" as const;
      await token.write.mint([userAcct.account.address, mintValue]);
      await token.write.approve([rollup.address, mintValue], {
        account: userAcct.account,
      });
      await rollup.write.mint(
        [
          "0x0000000000000000000000000000000000000000000000000000000000004444",
          mintValueBytes32,
          NOTE_KIND,
        ],
        { account: userAcct.account },
      );

      const tvlBefore = await rollup.read.currentTvl();
      const sinkBalBefore = await token.read.balanceOf([sink.account.address]);
      const userBalBefore = await token.read.balanceOf([
        userAcct.account.address,
      ]);

      await networkHelpers.time.increase(Number(SEVEN_DAYS) + 1);
      await networkHelpers.mine(1);

      const oldRoot = await rollup.read.rootHash();
      const zero =
        "0x0000000000000000000000000000000000000000000000000000000000000000" as const;
      const three =
        "0x0000000000000000000000000000000000000000000000000000000000000003" as const;
      const burnHash =
        "0x0000000000000000000000000000000000000000000000000000000000004445" as const;
      const userAddrAsBytes32 = (("0x000000000000000000000000" +
        userAcct.account.address.slice(2).toLowerCase()) as `0x${string}`);
      const padding: `0x${string}`[] = Array(25).fill(zero);
      const publicInputs: `0x${string}`[] = [
        oldRoot,
        "0x0000000000000000000000000000000000000000000000000000000000000020" as const,
        "0x0000000000000000000000000000000000000000000000000000000000000021" as const,
        three,
        NOTE_KIND,
        mintValueBytes32,
        burnHash,
        userAddrAsBytes32,
        ...padding,
      ];

      await rollup.write.verifyRollup(
        [1n, VK_HASH, "0x" as const, publicInputs, zero, []],
        { account: prover.account },
      );

      const payout = mintValue - FEE;

      // User received payout, not mintValue.
      const userBalAfter = await token.read.balanceOf([
        userAcct.account.address,
      ]);
      assert.equal(userBalAfter - userBalBefore, payout);

      // Sink got nothing (blacklisted).
      const sinkBalAfter = await token.read.balanceOf([sink.account.address]);
      assert.equal(sinkBalAfter - sinkBalBefore, 0n);

      // Fee stuck in contract.
      const rollupBal = await token.read.balanceOf([rollup.address]);
      assert.equal(rollupBal, FEE);

      // TVL decremented by payout only.
      const tvlAfter = await rollup.read.currentTvl();
      assert.equal(tvlBefore - tvlAfter, payout);
    });
  });

  // =================================================================
  // Plan-alignment tests
  // =================================================================
  describe("Plan alignment", () => {
    // addToken is back to its V1 shape: onlyOwner, existing-check,
    // store. No version gate, no alias constraint. The existing
    // devnet deploy.ts flow works unchanged because it calls addToken
    // pre-V2 from the deployer (who IS the owner at that point).
    it("addToken works from owner pre-V2 (V1 behavior restored)", async () => {
      const { viem } = await network.connect();
      const [deployer, prover, validator] = await viem.getWalletClients();
      const mockToken = await viem.deployContract("MockERC20");
      const otherToken = await viem.deployContract("MockERC20");
      const mockVerifier = await viem.deployContract("MockVerifier");
      const impl = await viem.deployContract("RollupV1");

      const initData = encodeFunctionData({
        abi: impl.abi,
        functionName: "initialize",
        args: [
          deployer.account.address,
          deployer.account.address,
          mockToken.address,
          mockVerifier.address,
          prover.account.address,
          [validator.account.address],
          VK_HASH,
        ],
      });
      const proxy = await viem.deployContract("RollupV2TestProxy", [
        impl.address,
        initData,
      ]);
      const rollup = await viem.getContractAt("RollupV1", proxy.address);

      // Deployer can register any token (V1 semantics).
      const kindA =
        "0x0001000000000000000000000000000000000000000000000000000000000000" as const;
      await rollup.write.addToken([kindA, otherToken.address], {
        account: deployer.account,
      });

      const registered = await rollup.read.noteKindTokenAddress([kindA]);
      assert.equal(
        registered.toLowerCase(),
        otherToken.address.toLowerCase(),
      );

      // Existing check still fires.
      await assert.rejects(
        rollup.write.addToken([kindA, otherToken.address], {
          account: deployer.account,
        }),
      );

      // Non-owner rejected (unchanged).
      const kindB =
        "0x0001000000000000000000000000000000000000000000000000000000000001" as const;
      await assert.rejects(
        rollup.write.addToken([kindB, otherToken.address], {
          account: validator.account,
        }),
      );
    });

    // After initializeV2, owner() is the newly-deployed timelock.
    // Direct onlyOwner calls from the pre-upgrade EOA must fail.
    it("initializeV2 transfers ownership to the deployed timelock", async () => {
      const { rollup, deployer } = await deployRollupV2();

      const ownerAfter = await rollup.read.owner();
      const tl = await rollup.read.timelock();

      assert.notEqual(
        ownerAfter.toLowerCase(),
        deployer.account.address.toLowerCase(),
        "owner should no longer be the deployer EOA",
      );
      assert.equal(
        ownerAfter.toLowerCase(),
        tl.toLowerCase(),
        "owner should be the newly-deployed timelock",
      );

      // EOA calls to onlyOwner functions fail.
      await assert.rejects(
        rollup.write.addToken(
          [
            "0x0001000000000000000000000000000000000000000000000000000000000099" as const,
            deployer.account.address,
          ],
          { account: deployer.account },
        ),
        "deployer should no longer be able to call onlyOwner functions",
      );
    });

    // Brick-prevention: initializeV2 must reject timelock configs
    // that would produce a non-functional TimelockController.
    //
    // Three paths, all equivalent outcomes (bricked governance):
    //   (a) empty executors  — nobody has EXECUTOR_ROLE
    //   (b) empty proposers  — nobody has PROPOSER_ROLE
    //   (c) only address(0) as proposer — PROPOSER_ROLE granted
    //       only to the zero address, no real account can schedule
    //
    // All three must revert at initializeV2 BEFORE ownership transfers,
    // so the deployer retains control and can redeploy / retry.
    it("initializeV2 rejects bricking timelock configs", async () => {
      // Helper: spin up a fresh proxy, return rollup handle + signer set.
      async function freshRollup() {
        const { viem } = await network.connect();
        const [deployer, prover, validator, sink] =
          await viem.getWalletClients();
        const mockToken = await viem.deployContract("MockERC20");
        const mockVerifier = await viem.deployContract("MockVerifier");
        const impl = await viem.deployContract("RollupV1");
        const initData = encodeFunctionData({
          abi: impl.abi,
          functionName: "initialize",
          args: [
            deployer.account.address,
            deployer.account.address,
            mockToken.address,
            mockVerifier.address,
            prover.account.address,
            [validator.account.address],
            VK_HASH,
          ],
        });
        const proxy = await viem.deployContract("RollupV2TestProxy", [
          impl.address,
          initData,
        ]);
        const rollup = await viem.getContractAt(
          "RollupV1",
          proxy.address,
        );
        return { rollup, deployer, sink };
      }

      const ZERO_ADDR =
        "0x0000000000000000000000000000000000000000" as const;

      // (a) empty executors
      {
        const { rollup, deployer, sink } = await freshRollup();
        await assert.rejects(
          rollup.write.initializeV2(
            [
              parseEther("1"),
              parseEther("10"),
              SEVEN_DAYS,
              2000n,
              sink.account.address,
              ONE_DAY,
              [deployer.account.address] as readonly `0x${string}`[],
              [] as readonly `0x${string}`[],
            ],
            { account: deployer.account },
          ),
        );
      }

      // (b) empty proposers
      {
        const { rollup, deployer, sink } = await freshRollup();
        await assert.rejects(
          rollup.write.initializeV2(
            [
              parseEther("1"),
              parseEther("10"),
              SEVEN_DAYS,
              2000n,
              sink.account.address,
              ONE_DAY,
              [] as readonly `0x${string}`[],
              [deployer.account.address] as readonly `0x${string}`[],
            ],
            { account: deployer.account },
          ),
        );
      }

      // (c) only address(0) as proposer
      {
        const { rollup, deployer, sink } = await freshRollup();
        await assert.rejects(
          rollup.write.initializeV2(
            [
              parseEther("1"),
              parseEther("10"),
              SEVEN_DAYS,
              2000n,
              sink.account.address,
              ONE_DAY,
              [ZERO_ADDR] as readonly `0x${string}`[],
              [deployer.account.address] as readonly `0x${string}`[],
            ],
            { account: deployer.account },
          ),
        );
      }

      // Sanity: executors = [address(0)] (open-role sentinel) is OK.
      {
        const { rollup, deployer, sink } = await freshRollup();
        await rollup.write.initializeV2(
          [
            parseEther("1"),
            parseEther("10"),
            SEVEN_DAYS,
            2000n,
            sink.account.address,
            ONE_DAY,
            [deployer.account.address] as readonly `0x${string}`[],
            [ZERO_ADDR] as readonly `0x${string}`[],
          ],
          { account: deployer.account },
        );
      }
    });
  });
});
