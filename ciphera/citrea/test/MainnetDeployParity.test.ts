// MainnetDeployParity.test.ts
//
// Exercises the same initialize → ProxyAdmin handoff → readback sequence
// that scripts/deploy.ts runs in production, but in-process against an
// ephemeral hardhat network.

import { describe, it } from "node:test";
import assert from "node:assert/strict";
import hre from "hardhat";
import { encodeFunctionData, getAddress, parseEther, zeroAddress, zeroHash } from "viem";

const EIP1967_ADMIN_STORAGE_SLOT =
  "0xb53127684a568b3173ae13b9f8a6016e243e63b6e8ee1178d6a717850b5d6103";

const ONE_HOUR = 3_600n;
const SEVEN_DAYS = 7n * 24n * ONE_HOUR;
const SATS_TO_WEI = 10_000_000_000n;
const VK_HASH =
  "0x0070170dcc7ad428aceab8724ef8ea429eb8baa18a1dc7f687d5ec3536f11fcc" as const;

function deriveNoteKind(chainId: number, tokenAddress: string): `0x${string}` {
  // Layout: [2 kind][6 zero][2 chainId][20 address][2 zero] = 32 bytes.
  // Matches scripts/deploy.ts::deriveInitialNoteKind and the noir circuit
  // convention.
  const prefix = "0002000000000000" + chainId.toString(16).padStart(4, "0");
  const tokenHex = tokenAddress.slice(2).toLowerCase().padStart(40, "0");
  return ("0x" + prefix + tokenHex + "0000").toLowerCase() as `0x${string}`;
}

function readAddressFromSlot(slot: `0x${string}`): `0x${string}` {
  assert.ok(slot.length >= 66, `invalid slot value: ${slot}`);
  return getAddress("0x" + slot.slice(26));
}

describe("Mainnet deploy parity", () => {
  it("matches deploy.ts invariants end-to-end", async () => {
    const { viem } = await hre.network.connect();
    const [owner, prover, validator, sink, burner] =
      await viem.getWalletClients();
    const publicClient = await viem.getPublicClient();

    const token = await viem.deployContract("MockERC20");
    const verifier = await viem.deployContract("MockVerifier");
    const impl = await viem.deployContract("RollupV1");

    const chainId = await publicClient.getChainId();
    const noteKind = deriveNoteKind(chainId, token.address);

    const perMintCap = parseEther("0.001");
    const globalTvlCap = parseEther("10");
    const burnFee = 300n * SATS_TO_WEI;

    const initData = encodeFunctionData({
      abi: impl.abi,
      functionName: "initialize",
      args: [
        owner.account.address,
        burner.account.address,
        token.address,
        noteKind,
        verifier.address,
        prover.account.address,
        [validator.account.address],
        VK_HASH,
        perMintCap,
        globalTvlCap,
        SEVEN_DAYS,
        burnFee,
        sink.account.address,
        ONE_HOUR,
        [owner.account.address],
        [zeroAddress],
      ],
    });

    const proxy = await viem.deployContract("RollupTestProxy", [
      impl.address,
      owner.account.address,
      initData,
    ]);

    const adminSlot = (await publicClient.getStorageAt({
      address: proxy.address,
      slot: EIP1967_ADMIN_STORAGE_SLOT,
    })) as `0x${string}`;
    const proxyAdminAddr = readAddressFromSlot(adminSlot);
    assert.notEqual(proxyAdminAddr.toLowerCase(), zeroAddress);

    const rollup = await viem.getContractAt("RollupV1", proxy.address);

    const timelockAddr = (await rollup.read.timelock()) as `0x${string}`;
    const rollupOwner = (await rollup.read.owner()) as `0x${string}`;
    assert.equal(rollupOwner.toLowerCase(), timelockAddr.toLowerCase());

    assert.equal(
      ((await rollup.read.token()) as string).toLowerCase(),
      token.address.toLowerCase(),
    );
    assert.equal(
      ((await rollup.read.zkVerifierKeys([0n])) as string).toLowerCase(),
      VK_HASH,
    );
    assert.equal(await rollup.read.perMintCap(), perMintCap);
    assert.equal(await rollup.read.globalTvlCap(), globalTvlCap);
    assert.equal(await rollup.read.openProvingDelay(), SEVEN_DAYS);
    assert.equal(await rollup.read.burnFee(), burnFee);
    assert.equal(
      ((await rollup.read.feeSink()) as string).toLowerCase(),
      sink.account.address.toLowerCase(),
    );
    assert.equal(
      await rollup.read.burnSubstitutors([owner.account.address]),
      true,
    );

    // ProxyAdmin handoff (deploy.ts does this after init). Use a minimal ABI
    // since ProxyAdmin isn't compiled directly into this repo.
    const proxyAdminAbi = [
      { type: "function", name: "owner", inputs: [], outputs: [{ type: "address" }], stateMutability: "view" },
      { type: "function", name: "transferOwnership", inputs: [{ type: "address" }], outputs: [], stateMutability: "nonpayable" },
    ] as const;
    const ownerBefore = (await publicClient.readContract({
      address: proxyAdminAddr,
      abi: proxyAdminAbi,
      functionName: "owner",
    })) as `0x${string}`;
    assert.equal(
      ownerBefore.toLowerCase(),
      owner.account.address.toLowerCase(),
    );
    const transferHash = await owner.writeContract({
      address: proxyAdminAddr,
      abi: proxyAdminAbi,
      functionName: "transferOwnership",
      args: [timelockAddr],
    });
    await publicClient.waitForTransactionReceipt({ hash: transferHash });
    const ownerAfter = (await publicClient.readContract({
      address: proxyAdminAddr,
      abi: proxyAdminAbi,
      functionName: "owner",
    })) as `0x${string}`;
    assert.equal(ownerAfter.toLowerCase(), timelockAddr.toLowerCase());

    void zeroHash;
  });

  it("rejects noteKind whose address slice does not match token", async () => {
    const { viem } = await hre.network.connect();
    const [owner, prover, validator, sink, burner] =
      await viem.getWalletClients();

    const token = await viem.deployContract("MockERC20");
    const verifier = await viem.deployContract("MockVerifier");
    const impl = await viem.deployContract("RollupV1");

    // 32-byte word whose bytes 10..29 are NOT token.address (deadbeef pad).
    const bogusNoteKind =
      "0x0002000000000000101200000000000000000000000000000000deadbeef0000" as const;

    const initData = encodeFunctionData({
      abi: impl.abi,
      functionName: "initialize",
      args: [
        owner.account.address,
        burner.account.address,
        token.address,
        bogusNoteKind,
        verifier.address,
        prover.account.address,
        [validator.account.address],
        VK_HASH,
        parseEther("0.001"),
        parseEther("10"),
        SEVEN_DAYS,
        300n * SATS_TO_WEI,
        sink.account.address,
        ONE_HOUR,
        [owner.account.address],
        [zeroAddress],
      ],
    });

    await assert.rejects(async () => {
      await viem.deployContract("RollupTestProxy", [
        impl.address,
        owner.account.address,
        initData,
      ]);
    });
  });
});
