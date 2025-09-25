import { expect } from "chai";
import { ethers } from "hardhat";
import { HardhatEthersSigner } from "@nomicfoundation/hardhat-ethers/signers";

describe("RollupV1 Debug Tests", function () {
  let rollup: any;
  let usdc: any;
  let mockVerifier: any;
  let owner: HardhatEthersSigner;
  let prover: HardhatEthersSigner;
  let validators: HardhatEthersSigner[];

  const EMPTY_MERKLE_ROOT = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
  const VERIFIER_KEY_HASH = "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";

  beforeEach(async function () {
    [owner, prover, ...validators] = await ethers.getSigners();

    // Deploy a mock USDC contract
    const MockUSDC = await ethers.getContractFactory("MockUSDC");
    usdc = await MockUSDC.deploy();

    // Deploy a mock verifier
    const MockVerifier = await ethers.getContractFactory("MockVerifier");
    mockVerifier = await MockVerifier.deploy();

    // Deploy RollupV1 (specify the rollup2 version)
    const RollupV1Factory = await ethers.getContractFactory("contracts/rollup2/RollupV1.sol:RollupV1");
    rollup = await RollupV1Factory.deploy();
  });

  it("should deploy contracts successfully", async function () {
    expect(await usdc.getAddress()).to.be.a('string');
    expect(await mockVerifier.getAddress()).to.be.a('string');
    expect(await rollup.getAddress()).to.be.a('string');
  });

  it("should fail initialization due to disabled initializers", async function () {
    const initialValidators = validators.slice(0, 3).map(v => v.address);
    
    // This should fail because the constructor calls _disableInitializers()
    await expect(rollup.initialize(
      owner.address,
      await usdc.getAddress(),
      await mockVerifier.getAddress(),
      prover.address,
      initialValidators,
      VERIFIER_KEY_HASH
    )).to.be.revertedWithCustomError(rollup, "InvalidInitialization");
  });
});