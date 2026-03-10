import { HardhatEthersSigner } from "@nomicfoundation/hardhat-ethers/signers";
import { expect } from "chai";
import { ethers } from "hardhat";

describe("SocialRecovery", function () {
  let owner: HardhatEthersSigner;
  let user: HardhatEthersSigner;
  let nonOwner: HardhatEthersSigner;
  let socialRecovery: any;

  beforeEach(async function () {
    [owner, user, nonOwner] = await ethers.getSigners();

    const SocialRecovery = await ethers.getContractFactory("SocialRecovery");
    socialRecovery = await SocialRecovery.deploy(owner.address);
    await socialRecovery.waitForDeployment();
  });

  describe("Guardian CID functionality", function () {
    it("Should add guardian successfully", async function () {
      await socialRecovery.addGuardianCID(
        user.address,
        "Password",
        "guardian-data",
      );

      const config = await socialRecovery.getGuardianConfig(user.address);
      expect(config.enabled).to.equal(true);
      expect(config.guardianCount).to.equal(1);
      expect(config.threshold).to.equal(1);
    });

    it("Should not allow duplicate guardians", async function () {
      await socialRecovery.addGuardianCID(user.address, "Password", "password-1");

      await expect(
        socialRecovery.addGuardianCID(user.address, "Password", "password-1"),
      ).to.be.revertedWith("Guardian Already exists");
    });
  });

  describe("Child Lit Action CID functionality", function () {
    it("Should return empty string initially", async function () {
      const cid = await socialRecovery.getChildLitActionCID();
      expect(cid).to.equal("");
    });

    it("Should set child lit action CID successfully", async function () {
      const testCID = "QmTest123456789abcdef";
      
      await expect(socialRecovery.setChildLitActionCID(testCID))
        .to.emit(socialRecovery, "ChildLitActionCIDUpdated")
        .withArgs("", testCID);

      const cid = await socialRecovery.getChildLitActionCID();
      expect(cid).to.equal(testCID);
    });

    it("Should update child lit action CID successfully", async function () {
      const firstCID = "QmFirst123456789abcdef";
      const secondCID = "QmSecond123456789abcdef";
      
      await socialRecovery.setChildLitActionCID(firstCID);
      
      await expect(socialRecovery.setChildLitActionCID(secondCID))
        .to.emit(socialRecovery, "ChildLitActionCIDUpdated")
        .withArgs(firstCID, secondCID);

      const cid = await socialRecovery.getChildLitActionCID();
      expect(cid).to.equal(secondCID);
    });

    it("Should only allow owner to set child lit action CID", async function () {
      const testCID = "QmTest123456789abcdef";
      
      await expect(
        socialRecovery.connect(nonOwner).setChildLitActionCID(testCID)
      ).to.be.revertedWithCustomError(socialRecovery, "OwnableUnauthorizedAccount")
        .withArgs(nonOwner.address);
    });

    it("Should reject empty CID", async function () {
      await expect(
        socialRecovery.setChildLitActionCID("")
      ).to.be.revertedWith("Child Lit Action CID cannot be empty");
    });

    it("Should maintain child lit action CID separate from guardian functionality", async function () {
      const testCID = "QmTest123456789abcdef";
      
      // Set child lit action CID
      await socialRecovery.setChildLitActionCID(testCID);
      
      // Add a guardian
      await socialRecovery.addGuardianCID(
        user.address,
        "Password",
        "guardian-data",
      );
      
      // Both should work independently
      const cid = await socialRecovery.getChildLitActionCID();
      expect(cid).to.equal(testCID);
      
      const config = await socialRecovery.getGuardianConfig(user.address);
      expect(config.enabled).to.equal(true);
      expect(config.guardianCount).to.equal(1);
    });
  });
});
