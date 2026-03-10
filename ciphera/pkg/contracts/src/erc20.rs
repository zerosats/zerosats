use crate::Client;
use crate::error::Result;
use ethereum_types::U64;
use sha3::{Digest, Keccak256};
use testutil::eth::EthNode;
use web3::{
    contract::{Contract, tokens::Tokenize},
    signing::{Key, SecretKey, SecretKeyRef},
    transports::Http,
    types::{Address, FilterBuilder, H256, U256},
};

#[derive(Debug, Clone)]
pub struct ERC20Contract {
    client: Client,
    contract: Contract<Http>,
    signer: SecretKey,
    signer_address: Address,
    address: Address,
    /// The ethereum block height used for all contract calls.
    /// If None, the latest block is used.
    block_height: Option<U64>,
}

impl ERC20Contract {
    pub fn new(
        client: Client,
        contract: Contract<Http>,
        signer: SecretKey,
        address: Address,
    ) -> Self {
        let signer_address = Key::address(&SecretKeyRef::new(&signer));

        Self {
            client,
            contract,
            signer,
            signer_address,
            address,
            block_height: None,
        }
    }

    pub fn at_height(&self, block_height: u64) -> Self {
        Self {
            block_height: Some(U64::from(block_height)),
            ..self.clone()
        }
    }

    pub fn address(&self) -> Address {
        self.address
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub async fn load(
        client: Client,
        erc20_contract_addr: &str,
        signer: SecretKey,
    ) -> Result<Self> {
        let contract_json =
            include_str!("../../../citrea/openzeppelin-contracts/token/ERC20/IERC20.json");
        let contract = client.load_contract_from_str(erc20_contract_addr, contract_json)?;

        Ok(Self::new(
            client,
            contract,
            signer,
            erc20_contract_addr.parse()?,
        ))
    }

    pub async fn from_eth_node(eth_node: &EthNode, signer: SecretKey) -> Result<Self> {
        let erc20_addr = "5fbdb2315678afecb367f032d93f642f64180aa3";

        let client = Client::from_eth_node(eth_node);
        Self::load(client, erc20_addr, signer).await
    }

    pub async fn call(&self, func: &str, params: impl Tokenize + Clone) -> Result<H256> {
        self.client
            .call(
                &self.contract,
                func,
                params,
                &self.signer,
                self.signer_address,
            )
            .await
    }

    #[tracing::instrument(err, ret, skip(self))]
    pub async fn mint(&self, to: Address, amount: u128) -> Result<H256> {
        self.call("mint", (to, amount)).await
    }

    #[tracing::instrument(err, ret, skip(self))]
    pub async fn transfer(&self, to: Address, amount: u128) -> Result<H256> {
        self.call("transfer", (to, amount)).await
    }

    // Query allowance
    #[tracing::instrument(err, ret, skip(self))]
    pub async fn allowance(&self, owner: Address, spender: Address) -> Result<U256> {
        let allowance = self
            .client
            .query(
                &self.contract,
                "allowance",
                (owner, spender),
                None,
                Default::default(),
                self.block_height.map(|x| x.into()),
            )
            .await?;

        Ok(allowance)
    }

    #[tracing::instrument(err, ret, skip(self))]
    pub async fn balance(&self, owner: Address) -> Result<U256> {
        let balance = self
            .client
            .query(
                &self.contract,
                "balanceOf",
                (owner,),
                None,
                Default::default(),
                self.block_height.map(|x| x.into()),
            )
            .await?;
        Ok(balance)
    }

    /// Check if an EIP-3009 authorization has been used by scanning the AuthorizationUsed event.
    /// Many USDC implementations emit: AuthorizationUsed(address indexed authorizer, bytes32 indexed nonce)
    /// Returns the transaction hash if found.
    #[tracing::instrument(err, ret, skip(self))]
    pub async fn authorization_used_txn(
        &self,
        authorizer: Address,
        nonce: H256,
    ) -> Result<Option<H256>> {
        // keccak256("AuthorizationUsed(address,bytes32)")
        let topic0 = H256::from_slice(&Keccak256::digest(b"AuthorizationUsed(address,bytes32)"));
        let authorizer_h = H256::from(authorizer);

        let filter = FilterBuilder::default()
            .address(vec![self.address])
            .from_block(web3::types::BlockNumber::Earliest)
            .to_block(web3::types::BlockNumber::Latest)
            .topics(
                Some(vec![topic0]),
                Some(vec![authorizer_h]),
                Some(vec![nonce]),
                None,
            )
            .build();

        let logs = self.client.logs(filter).await?;
        Ok(logs.into_iter().filter_map(|l| l.transaction_hash).next())
    }

    /// Approve contract to spend USDC on behalf of the user
    #[tracing::instrument(err, ret, skip(self))]
    pub async fn approve_max(&self, from: Address) -> Result<H256> {
        self.call("approve", (from, web3::types::U256::MAX)).await
    }

    #[tracing::instrument(err, ret, skip(self))]
    pub async fn approve(&self, from: Address, amount: u128) -> Result<H256> {
        self.call("approve", (from, amount)).await
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use crate::tests::get_env;

//     // TODO: fix this test
//     #[tokio::test]
//     async fn test_approve() {
//         let env = get_env();
//         let allowance = env
//             .erc20_contract
//             .allowance(env.evm_address, env.rollup_contract_addr)
//             .await
//             .unwrap();

//         assert_eq!(allowance, U256::max_value());
//     }
// }
