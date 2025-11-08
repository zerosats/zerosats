use element::Element;
use rand::{RngCore, rngs::OsRng};
use zk_primitives::{InputNote, MerklePath, Note, get_address_for_private_key};

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

    pub fn new_note(&self, amount: u64, contract: Element) -> Note {
        Note {
            kind: Element::new(2),
            value: Element::new(amount),
            address: self.address(),
            contract,
            psi: Element::new(0),
        }
    }
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