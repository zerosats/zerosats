use crate::domain::Quote;
use async_trait::async_trait;
use barretenberg::Prove;
use element::Element;
use zk_primitives::{InputNote, Note, TimeLock, TimeProof, Utxo, UtxoProof};

/// Builds and proves the SlowBurn UTXO that redeems a quote's HTLC escrow.
///
/// In production the prover talks to `barretenberg` (`bb_rs` backend); in
/// tests the trait can be mocked with a stub returning a canned proof.
#[async_trait]
pub trait BurnProver: Send + Sync {
    async fn build_and_prove(
        &self,
        quote: &Quote,
        preimage: [u8; 32],
        service_evm_address: Element,
    ) -> eyre::Result<UtxoProof>;
}

#[derive(Default)]
pub struct LocalBurnProver;

#[async_trait]
impl BurnProver for LocalBurnProver {
    async fn build_and_prove(
        &self,
        quote: &Quote,
        preimage: [u8; 32],
        service_evm_address: Element,
    ) -> eyre::Result<UtxoProof> {
        let lock = TimeLock {
            zero_block: quote.zero_block,
            n_blocks: quote.n_blocks,
        };
        let escrow_note = Note::new_htlc_for_recipient(
            quote.user_address,
            &lock,
            quote.payment_hash,
            quote.note_kind,
            quote.amount,
        );

        let mut input = InputNote::new_htlc_claim(escrow_note, preimage);
        // The HTLC hash-claim branch does not require a PoW witness, but
        // the circuit layout always carries a time_proof field. Defaults
        // are zeros, which the circuit ignores on this branch.
        input.time_proof = TimeProof::default();

        let utxo = Utxo::new_burn_no_sub(
            [input, InputNote::padding_note()],
            service_evm_address,
        );

        // Heavy proving call. Runs synchronously on a blocking thread so
        // the async settlement worker does not block. The raw barretenberg
        // error is `Box<dyn StdError>` which is !Send, so stringify it
        // before crossing the thread boundary.
        let utxo_clone = utxo.clone();
        let proof = tokio::task::spawn_blocking(move || {
            utxo_clone.prove().map_err(|e| format!("{e:?}"))
        })
        .await
        .map_err(|e| eyre::eyre!("prove task panicked: {e}"))?
        .map_err(|e| eyre::eyre!("prove failed: {e}"))?;
        Ok(proof)
    }
}
