use element::Element;
use rand::{RngCore, rngs::OsRng};
use zk_primitives::{InputNote, MerklePath, Note, get_address_for_private_key, generate_note_kind_bridge_evm};
use contracts::{Address, RollupContract, SecretKey, USDCContract, util::convert_h160_to_element};
use web3::types::H160;

// Reused from payy/pkg/contracts/src/tests/test_rollup.rs
//
// =====================================================================
// Wallet & helpers
// =====================================================================

#[derive(Clone, Copy, Debug)]
pub struct Wallet {
    /// *Private* key in the zk‑Primitive sense – **NOT** an ECDSA key!
    pub pk: Element,
}

impl Wallet {
    /// Create a wallet from an explicit private key.
    #[expect(unused)]
    pub fn new(pk: Element) -> Self {
        Self { pk }
    }

    /// Create a wallet with a random 256‑bit private key.
    pub fn random() -> Self {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        Self {
            pk: Element::from_be_bytes(bytes),
        }
    }

    /// Derive the *address* (Poseidon‑hashed) that the circuits use.
    pub fn address(&self) -> Element {
        get_address_for_private_key(self.pk)
    }

    pub fn new_note(&self, amount: u64, chain: u64, token: H160) -> InputNote {
        //Note::new_with_psi(address, value, psi);
        let contract = generate_note_kind_bridge_evm(chain, token);
        InputNote::new(        Note {
            kind: Element::new(2),
            value: Element::new(amount),
            address: self.address(),
            contract,
            psi: Element::new(0),
        }, self.pk)
    }

    #[expect(unused)]
    pub fn mint() {

    }
/*    #[expect(unused)]
    fn mint_with_note<'m, 't>(
        rollup: &'m RollupContract,
        _usdc: &'m USDCContract,
        server: &'t Server,
        note: Note,
    ) -> (
        impl Future<Output = Result<(), contracts::Error>> + 'm,
        impl Future<Output = Result<TransactionResp, Error>> + 't,
    ) {
        let output_notes = [note.clone(), Note::padding_note()];
        let utxo = zk_primitives::Utxo::new_mint(output_notes.clone());
        let proof = utxo.prove().unwrap();

        (
            async move {
                let tx = rollup
                    .mint(&utxo.mint_hash(), &note.value, &note.contract)
                    .await?;

                while rollup
                    .client
                    .client()
                    .eth()
                    .transaction_receipt(tx)
                    .await
                    .unwrap()
                    .is_none_or(|r| r.block_number.is_none())
                {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }

                Ok(())
            },
            async move { server.transaction(&proof).await },
        )
    }*/
}

// =====================================================================
// A note bundled with the wallet that created / owns it
// =====================================================================

#[derive(Clone, Debug)]
pub struct WalletNote {
    note: Note,
    pub wallet: Wallet,
}

impl WalletNote {
    fn new(wallet: Wallet, note: Note) -> Self {
        Self { note, wallet }
    }

    /// Raw commitment (32‑byte field element).
    #[expect(unused)]
    pub fn commitment(&self) -> Element {
        self.note.commitment()
    }

    /// Borrow the underlying `Note`.
    pub fn note(&self) -> Note {
        self.note.clone()
    }
}