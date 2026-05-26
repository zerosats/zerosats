import { HardhatUserConfig } from "hardhat/config";
import hardhatToolboxViemPlugin from "@nomicfoundation/hardhat-toolbox-viem";
import { configVariable } from "hardhat/config";

const config: HardhatUserConfig = {
  plugins: [hardhatToolboxViemPlugin],

  solidity: {
    compilers: [
      {
        version: "0.8.28",
        settings: {
          viaIR: true,
          optimizer: { enabled: true, runs: 200 },
        },
      },
    ],
  },

  networks: {
    hardhatMainnet: {
      type: "edr-simulated",
      chainType: "l1",
    },
    citreaDevnet: {
      type: "http",
      chainId: 5655,
      url: "http://localhost:12345",
      accounts: { mnemonic: configVariable("MNEMONIC") },
    },
    citreaTestnet: {
      type: "http",
      chainId: 5115,
      // Override the public RPC by setting CITREA_TESTNET_RPC_URL in your env.
      url: configVariable("CITREA_TESTNET_RPC_URL"),
      accounts: { mnemonic: configVariable("MNEMONIC") },
    },
    citreaMainnet: {
      type: "http",
      chainId: 4114,
      // Mainnet RPC is mandatory and operator-supplied. Suggested template:
      //   https://citrea-mainnet.g.alchemy.com/v2/<ALCHEMY_KEY>
      url: configVariable("CITREA_MAINNET_RPC_URL"),
      accounts: { mnemonic: configVariable("MNEMONIC") },
    },
  },

  chainDescriptors: {
    5115: {
      name: "citreaTestnet",
      blockExplorers: {
        blockscout: {
          name: "Citrea Testnet Explorer",
          url: "https://explorer.testnet.citrea.xyz",
          apiUrl: "https://explorer.testnet.citrea.xyz/api",
        },
      },
    },
    4114: {
      name: "citreaMainnet",
      blockExplorers: {
        blockscout: {
          name: "Citrea Mainnet Explorer",
          url: "https://explorer.mainnet.citrea.xyz",
          apiUrl: "https://explorer.mainnet.citrea.xyz/api",
        },
      },
    },
  },
};

export default config;