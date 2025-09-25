// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "@openzeppelin/contracts/access/Ownable.sol";

contract SocialRecovery is Ownable {

    struct GuardianEntry {
        bytes32 cidHash;
        bytes guardianValue;
    }

    struct GuardianConfig {
        uint256 threshold;
        bool enabled;
        GuardianEntry[] guardianCIDs;
        mapping(bytes32 => uint256) index;
        mapping(bytes32 => bool) exists;
    }


    mapping(address => GuardianConfig) public guardianConfigs;
    // This is the child/proxy lit action that will be called by the parent lit-action
    // for iteratively verifying all the guardians and checking threshold is met
    // For more info refer to the notion doc on Guardian Recovery
    string public childLitActionCID;

    event GuardianCIDAdded(address indexed user, string guardianCID);
    event ChildLitActionCIDUpdated(string oldCID, string newCID);

    constructor(address initialOwner) Ownable(initialOwner) {}

    function addGuardianCID(address user, string memory guardianCID, string memory guardianValue) external onlyOwner {
        GuardianConfig storage guardianConfig = guardianConfigs[user];

        if (guardianConfig.guardianCIDs.length == 0) {
            guardianConfig.enabled = true;
        }

        require(guardianConfig.enabled, "Guardian recovery disabled");
        require(bytes(guardianCID).length > 0, "Guardian CID cannot be empty");

       bytes32 cidHash = keccak256(bytes(guardianCID));

        bool exists =  guardianConfig.exists[cidHash]; 

        require(!exists, "Guardian Already exists");


        guardianConfig.index[cidHash] = guardianConfig.guardianCIDs.length;
        guardianConfig.exists[cidHash] = true;

        GuardianEntry memory newGuardian = GuardianEntry({
            cidHash: cidHash,
            guardianValue: bytes(guardianValue)
        });

        guardianConfig.guardianCIDs.push(newGuardian);


        if (guardianConfig.threshold == 0) {
            guardianConfig.threshold = 1;
        }

        emit GuardianCIDAdded(user, guardianCID);
    }

    function updateThreshold(address user, uint256 newThreshold) external onlyOwner {
        GuardianConfig storage config = guardianConfigs[user];
        require(config.guardianCIDs.length > 0, "No guardians configured");
        require(newThreshold > 0 && newThreshold <= config.guardianCIDs.length, "Invalid threshold");
        config.threshold = newThreshold;
    }


    function getGuardianConfig(address user) external view returns (
        uint256 threshold,
        bool enabled,
        uint256 guardianCount,
        GuardianEntry[] memory guardians
    ) {
        GuardianConfig storage config = guardianConfigs[user];
        return (
            config.threshold,
            config.enabled,
            config.guardianCIDs.length,
            config.guardianCIDs  
        );
    }

    function setChildLitActionCID(string memory newCID) external onlyOwner {
        require(bytes(newCID).length > 0, "Child Lit Action CID cannot be empty");
        
        string memory oldCID = childLitActionCID;
        childLitActionCID = newCID;
        
        emit ChildLitActionCIDUpdated(oldCID, newCID);
    }

    function getChildLitActionCID() external view returns (string memory) {
        return childLitActionCID;
    }

}

