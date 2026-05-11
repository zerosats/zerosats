use barretenberg::Prove;
use zk_primitives::{InputNote, Note, Utxo, UtxoProof};

/// Build and prove a send-kind UTXO transaction.
///
/// Takes two input notes (one may be the padding note) and two output notes
/// (likewise — one is typically the change note) and returns the snark-ready
/// [`UtxoProof`] for submission to a node.
pub fn prove_send(
    inputs: [InputNote; 2],
    outputs: [Note; 2],
) -> Result<UtxoProof, Box<dyn std::error::Error>> {
    Utxo::new_send(inputs, outputs).prove()
}
