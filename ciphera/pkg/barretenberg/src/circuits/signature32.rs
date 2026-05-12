use std::path::PathBuf;

use element::Base;
use lazy_static::lazy_static;
use noirc_abi::InputMap;
use noirc_artifacts::program::ProgramArtifact;
use noirc_driver::CompiledProgram;
use zk_primitives::{
    bytes_to_elements, Signature32, Signature32Proof, Signature32ProofBytes, Signature32PublicInput
};

use crate::{
    backend::DefaultBackend,
    circuits::get_bytecode_from_program,
    prove::prove,
    traits::{Prove, Verify},
    util::write_to_temp_file,
    verify::verify,
    Result,
};

const PROGRAM: &str = include_str!("../../../../fixtures/programs/signature32.json");
const KEY: &[u8] = include_bytes!("../../../../fixtures/keys/signature32_key");

lazy_static! {
    static ref PROGRAM_ARTIFACT: ProgramArtifact = serde_json::from_str(PROGRAM).unwrap();
    static ref PROGRAM_COMPILED: CompiledProgram = CompiledProgram::from(PROGRAM_ARTIFACT.clone());
    static ref PROGRAM_PATH: PathBuf = write_to_temp_file(PROGRAM.as_bytes(), ".json");
    static ref BYTECODE: Vec<u8> = get_bytecode_from_program(PROGRAM);
}

const SIGNATURE_PUBLIC_INPUTS_COUNT: usize = 2;
const SIGNATURE_PROOF_SIZE: usize = 508;

impl Prove for Signature32 {
    type Proof = Signature32Proof;
    type Result<Proof> = Result<Proof>;

    fn prove(&self) -> Self::Result<Self::Proof> {
        let inputs = InputMap::from(Signature32Input::from(self));

        let proof_bytes = prove::<DefaultBackend>(
            &PROGRAM_COMPILED,
            PROGRAM.as_bytes(),
            &BYTECODE,
            KEY,
            &inputs,
            false,
            false,
        )?;

        // Slice off the public inputs
        let public_inputs = proof_bytes[..SIGNATURE_PUBLIC_INPUTS_COUNT * 32].to_vec();
        let public_inputs = bytes_to_elements(&public_inputs);
        let raw_proof = proof_bytes[SIGNATURE_PUBLIC_INPUTS_COUNT * 32..].to_vec();

        assert_eq!(
            raw_proof.len(),
            SIGNATURE_PROOF_SIZE * 32,
            "Proof must be {SIGNATURE_PROOF_SIZE} elements of 32 bytes"
        );

        // Convert the proof bytes to a Signature32Proof
        let proof = Signature32Proof {
            proof: Signature32ProofBytes(raw_proof),
            public_inputs: Signature32PublicInput {
                address: public_inputs[0],
                message: public_inputs[1],
            },
        };

        Ok(proof)
    }
}

impl Verify for Signature32Proof {
    fn verify(&self) -> Result<()> {
        verify::<DefaultBackend>(KEY, &self.public_inputs.to_bytes(), &self.proof.0, false)
    }
}

#[derive(Debug, Clone)]
struct Signature32Input {
    preimage: [u8; 32],
    message: Base,
    message_hash: Base,
    address: Base,
}

impl From<&Signature32> for Signature32Input {
    fn from(signature: &Signature32) -> Self {
        Signature32Input {
            preimage: signature.preimage,
            message: signature.message.to_base(),
            message_hash: signature.message_hash().to_base(),
            address: signature.address().to_base(),
        }
    }
}

impl From<Signature32Input> for InputMap {
    fn from(input: Signature32Input) -> Self {
        let mut map = InputMap::new();

        map.insert(
            "preimage".to_owned(),
            noirc_abi::input_parser::InputValue::Vec(
                input
                    .preimage
                    .iter()
                    .map(|&b| noirc_abi::input_parser::InputValue::Field(Base::from(b as u128)))
                    .collect(),
            ),
        );
        map.insert(
            "message_hash".to_owned(),
            noirc_abi::input_parser::InputValue::Field(input.message_hash),
        );
        map.insert(
            "address".to_owned(),
            noirc_abi::input_parser::InputValue::Field(input.address),
        );
        map.insert(
            "message".to_owned(),
            noirc_abi::input_parser::InputValue::Field(input.message),
        );

        map
    }
}

#[cfg(test)]
mod tests {
    use element::Element;

    use super::*;

    #[test]
    fn test_signature_proof_generation_and_verification() {
        // Create a signature with a test preimage and message
        let preimage = [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
            17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
        ];
        let message = Element::from(100u64);
        let signature = Signature32 {
            preimage,
            message,
        };

        // Generate a proof for the signature
        let proof = signature.prove().unwrap();

        // Verify the proof
        proof.verify().expect("Verification failed");
    }

    #[test]
    fn test_secp_scalar_proof() {
        use musig2::secp::Scalar;

        let adaptor_secret: Scalar = "44477400e59c41025e4e18c4de244b90b14554dcdcbfa396ead4659aa6343249".parse().unwrap();

        // Convert Scalar to [u8; 32] preimage
        let preimage: [u8; 32] = adaptor_secret.serialize();

        let message = Element::from(100u64);
        let signature = Signature32 {
            preimage,
            message,
        };

        let proof = signature.prove().unwrap();

        proof.verify().expect("Verification failed");
    }

    #[test]
    fn test_adaptor_flow() {
        use musig2::secp::{MaybeScalar, Point, Scalar};
        use musig2::{AdaptorSignature, KeyAggContext, PartialSignature};

        let seckeys = [
            Scalar::from_slice(&[0x11; 32]).unwrap(),
            Scalar::from_slice(&[0x22; 32]).unwrap(),
            Scalar::from_slice(&[0x33; 32]).unwrap(),
        ];

        let pubkeys = [
            seckeys[0].base_point_mul(),
            seckeys[1].base_point_mul(),
            seckeys[2].base_point_mul(),
        ];

        let key_agg_ctx = KeyAggContext::new(pubkeys).unwrap();
        let aggregated_pubkey: Point = key_agg_ctx.aggregated_pubkey();

        let message = "danger, will robinson!";

        let adaptor_secret: Scalar = "44477400e59c41025e4e18c4de244b90b14554dcdcbfa396ead4659aa6343249".parse().unwrap();
        let adaptor_point = adaptor_secret.base_point_mul();

        use musig2::{AggNonce, SecNonce};

        let secnonces = [
            SecNonce::build([0x11; 32]).build(),
            SecNonce::build([0x22; 32]).build(),
            SecNonce::build([0x33; 32]).build(),
        ];

        let pubnonces = [
            secnonces[0].public_nonce(),
            secnonces[1].public_nonce(),
            secnonces[2].public_nonce(),
        ];

        let aggnonce = AggNonce::sum(&pubnonces);

        let partial_signatures: Vec<PartialSignature> = seckeys
            .into_iter()
            .zip(secnonces)
            .map(|(seckey, secnonce)| {
                musig2::adaptor::sign_partial(
                    &key_agg_ctx,
                    seckey,
                    secnonce,
                    &aggnonce,
                    adaptor_point,
                    &message,
                )
            })
            .map(|r| r.map_err(|e| Box::new(e) as Box<dyn std::error::Error>))
            .collect::<Result<Vec<_>>>()
            .expect("failed to create partial adaptor signatures");

        let adaptor_signature: AdaptorSignature = musig2::adaptor::aggregate_partial_signatures(
            &key_agg_ctx,
            &aggnonce,
            adaptor_point,
            partial_signatures.iter().copied(),
            &message,
        )
            .expect("failed to aggregate partial adaptor signatures");

        // Verify the adaptor signature is valid for the given adaptor point and pubkey.
        musig2::adaptor::verify_single(
            aggregated_pubkey,
            &adaptor_signature,
            &message,
            adaptor_point,
        )
            .expect("invalid aggregated adaptor signature");

        // Decrypt the signature with the adaptor secret.
        let valid_signature = adaptor_signature.adapt(adaptor_secret).unwrap();

        musig2::verify_single(
            aggregated_pubkey,
            valid_signature,
            &message,
        )
            .expect("invalid decrypted adaptor signature");

        // The decrypted signature and the adaptor signature allow an
        // observer to deduce the adaptor secret.
        let revealed: MaybeScalar = adaptor_signature
            .reveal_secret(&valid_signature)
            .expect("should compute adaptor secret from decrypted signature");

        // Convert Scalar to [u8; 32] preimage
        let preimage: [u8; 32] = revealed.serialize();

        let message = Element::from(100u64);

        let signature = Signature32 {
            preimage,
            message,
        };

        let proof = signature.prove().unwrap();

        proof.verify().expect("Verification failed");
    }

    #[test]
    fn test_abc_statechain_transfer() {
        use musig2::secp::{MaybeScalar, Point, Scalar};
        use musig2::{AdaptorSignature, KeyAggContext, PartialSignature};

        // === Key setup ===
        // A (statechain entity) and X (transitory key, known by B)
        let seckey_a = Scalar::from_slice(&[0xAA; 32]).unwrap();
        let seckey_x = Scalar::from_slice(&[0xBB; 32]).unwrap();
        // B (current owner) and C (new owner) signing keys for BC signature
        let seckey_c = Scalar::from_slice(&[0xDD; 32]).unwrap();

        // === Three adaptor secrets: #A, #B, #C ===
        let adaptor_secret_a: Scalar = "11477400e59c41025e4e18c4de244b90b14554dcdcbfa396ead4659aa6343249".parse().unwrap();
        let adaptor_secret_b: Scalar = "22477400e59c41025e4e18c4de244b90b14554dcdcbfa396ead4659aa6343249".parse().unwrap();
        let adaptor_secret_c: Scalar = "33477400e59c41025e4e18c4de244b90b14554dcdcbfa396ead4659aa6343249".parse().unwrap();

        let adaptor_point_a = adaptor_secret_a.base_point_mul();
        let adaptor_point_b = adaptor_secret_b.base_point_mul();
        let adaptor_point_c = adaptor_secret_c.base_point_mul();

        // Combined adaptor point #ABC = #A + #B + #C
        let adaptor_secret_abc = adaptor_secret_a + adaptor_secret_b + adaptor_secret_c;
        let adaptor_point_abc = adaptor_point_a + adaptor_point_b + adaptor_point_c;

        let statechain_message = "statechain transfer";

        // === Preparation Step 2: B and C create adaptor signature BC missing #ABC ===
        let pubkeys_bc = [
            seckey_x.base_point_mul(),
            seckey_c.base_point_mul(),
        ];
        let key_agg_ctx_bc = KeyAggContext::new(pubkeys_bc).unwrap();
        let aggregated_pubkey_bc: Point = key_agg_ctx_bc.aggregated_pubkey();

        let secnonces_bc = [
            musig2::SecNonce::build([0xCC; 32]).build(),
            musig2::SecNonce::build([0xDD; 32]).build(),
        ];
        let pubnonces_bc = [
            secnonces_bc[0].public_nonce(),
            secnonces_bc[1].public_nonce(),
        ];
        let aggnonce_bc = musig2::AggNonce::sum(&pubnonces_bc);

        let partial_sigs_bc: Vec<PartialSignature> = [seckey_x, seckey_c]
            .into_iter()
            .zip(secnonces_bc)
            .map(|(seckey, secnonce)| {
                musig2::adaptor::sign_partial(
                    &key_agg_ctx_bc,
                    seckey,
                    secnonce,
                    &aggnonce_bc,
                    adaptor_point_abc,
                    &statechain_message,
                )
            })
            .map(|r| r.map_err(|e| Box::new(e) as Box<dyn std::error::Error>))
            .collect::<Result<Vec<_>>>()
            .expect("failed to create BC partial adaptor signatures");

        let adaptor_sig_bc: AdaptorSignature = musig2::adaptor::aggregate_partial_signatures(
            &key_agg_ctx_bc,
            &aggnonce_bc,
            adaptor_point_abc,
            partial_sigs_bc.iter().copied(),
            &statechain_message,
        )
            .expect("failed to aggregate BC adaptor signatures");

        // B and C pass adaptor_sig_bc to A
        musig2::adaptor::verify_single(
            aggregated_pubkey_bc,
            &adaptor_sig_bc,
            &statechain_message,
            adaptor_point_abc,
        )
            .expect("invalid BC adaptor signature");

        // === Preparation Step 3: A and X create adaptor signature AX missing #ABC ===
        let message = "basechain transfer";
        let pubkeys_ax = [
            seckey_a.base_point_mul(),
            seckey_x.base_point_mul(),
        ];
        let key_agg_ctx_ax = KeyAggContext::new(pubkeys_ax).unwrap();
        let aggregated_pubkey_ax: Point = key_agg_ctx_ax.aggregated_pubkey();

        let secnonces_ax = [
            musig2::SecNonce::build([0xAA; 32]).build(),
            musig2::SecNonce::build([0xBB; 32]).build(),
        ];
        let pubnonces_ax = [
            secnonces_ax[0].public_nonce(),
            secnonces_ax[1].public_nonce(),
        ];
        let aggnonce_ax = musig2::AggNonce::sum(&pubnonces_ax);

        let partial_sigs_ax: Vec<PartialSignature> = [seckey_a, seckey_x]
            .into_iter()
            .zip(secnonces_ax)
            .map(|(seckey, secnonce)| {
                musig2::adaptor::sign_partial(
                    &key_agg_ctx_ax,
                    seckey,
                    secnonce,
                    &aggnonce_ax,
                    adaptor_point_abc,
                    &message,
                )
            })
            .map(|r| r.map_err(|e| Box::new(e) as Box<dyn std::error::Error>))
            .collect::<Result<Vec<_>>>()
            .expect("failed to create AX partial adaptor signatures");

        let adaptor_sig_ax: AdaptorSignature = musig2::adaptor::aggregate_partial_signatures(
            &key_agg_ctx_ax,
            &aggnonce_ax,
            adaptor_point_abc,
            partial_sigs_ax.iter().copied(),
            &message,
        )
            .expect("failed to aggregate AX adaptor signatures");

        // A and X pass adaptor_sig_ax to C
        musig2::adaptor::verify_single(
            aggregated_pubkey_ax,
            &adaptor_sig_ax,
            &message,
            adaptor_point_abc,
        )
            .expect("invalid AX adaptor signature");

        // === Transfer Step 1: C reveals #c to B ===
        // B now knows #b and #c, can compute #bc
        let adaptor_secret_bc = adaptor_secret_b + adaptor_secret_c;

        // === Transfer Step 2: B reveals #bc to A ===
        // A now knows #a and #bc, can compute #abc
        let adaptor_secret_abc_computed = adaptor_secret_a + adaptor_secret_bc;
        assert_eq!(adaptor_secret_abc_computed, adaptor_secret_abc);

        // === Transfer Step 3: A publishes signature BC, revealing #abc to C ===
        // A adapts the BC signature with the full secret #abc
        let valid_sig_bc = adaptor_sig_bc.adapt(adaptor_secret_abc).unwrap();
        // Decrypt the signature with the adaptor secret.

        musig2::verify_single(
            aggregated_pubkey_bc,
            valid_sig_bc,
            &statechain_message,
        )
            .expect("invalid decrypted BC signature");

        // C observes the published BC signature and extracts #abc
        let revealed_abc: MaybeScalar = adaptor_sig_bc
            .reveal_secret(&valid_sig_bc)
            .expect("should reveal #abc from BC signature");

        assert_eq!(revealed_abc, MaybeScalar::Valid(adaptor_secret_abc.unwrap()));

        // C knows #c and now knows #abc, so C can derive #ab = #abc - #c
        // More importantly, C can now adapt the AX signature
        let valid_sig_ax: musig2::LiftedSignature = adaptor_sig_ax.adapt(revealed_abc.unwrap()).unwrap();

        musig2::verify_single(
            aggregated_pubkey_ax,
            valid_sig_ax,
            &message,
        )
            .expect("invalid decrypted AX signature");

        // === Transfer Step 4: C now holds both valid signatures ===
        // C has valid_sig_ax (can spend the UTXO via AX path)
        // C has valid_sig_bc (the statechain record)
        // Transfer is complete.
    }

    #[test]
    fn test_zcash_note_adaptor() {
        use musig2::secp::{Point, Scalar};
        use musig2::{AdaptorSignature, KeyAggContext, PartialSignature};

        // === Key setup ===
        // A (statechain entity) and X (transitory key, known by B)
        let _seckey_a = Scalar::from_slice(&[0xAA; 32]).unwrap();
        let seckey_x = Scalar::from_slice(&[0xBB; 32]).unwrap();
        // B (current owner) and C (new owner) signing keys for BC signature
        let seckey_c = Scalar::from_slice(&[0xDD; 32]).unwrap();

        // === Three adaptor secrets: #B, #C ===
        let adaptor_secret_b: Scalar = "22477400e59c41025e4e18c4de244b90b14554dcdcbfa396ead4659aa6343249".parse().unwrap();
        let adaptor_secret_c: Scalar = "33477400e59c41025e4e18c4de244b90b14554dcdcbfa396ead4659aa6343249".parse().unwrap();

        let adaptor_point_b = adaptor_secret_b.base_point_mul();
        let adaptor_point_c = adaptor_secret_c.base_point_mul();

        // Combined adaptor point #BC = #B + #C
        let adaptor_secret_bc = adaptor_secret_b + adaptor_secret_c;
        let adaptor_point_bc = adaptor_point_b + adaptor_point_c;

        let message = "vault transfer";

        // === Preparation Step 2: B and C create adaptor signature BC missing #ABC ===
        let pubkeys_bc = [
            seckey_x.base_point_mul(),
            seckey_c.base_point_mul(),
        ];
        let key_agg_ctx_bc = KeyAggContext::new(pubkeys_bc).unwrap();
        let aggregated_pubkey_bc: Point = key_agg_ctx_bc.aggregated_pubkey();

        let secnonces_bc = [
            musig2::SecNonce::build([0xCC; 32]).build(),
            musig2::SecNonce::build([0xDD; 32]).build(),
        ];
        let pubnonces_bc = [
            secnonces_bc[0].public_nonce(),
            secnonces_bc[1].public_nonce(),
        ];
        let aggnonce_bc = musig2::AggNonce::sum(&pubnonces_bc);

        let partial_sigs_bc: Vec<PartialSignature> = [seckey_x, seckey_c]
            .into_iter()
            .zip(secnonces_bc)
            .map(|(seckey, secnonce)| {
                musig2::adaptor::sign_partial(
                    &key_agg_ctx_bc,
                    seckey,
                    secnonce,
                    &aggnonce_bc,
                    adaptor_point_bc,
                    &message,
                )
            })
            .map(|r| r.map_err(|e| Box::new(e) as Box<dyn std::error::Error>))
            .collect::<Result<Vec<_>>>()
            .expect("failed to create BC partial adaptor signatures");

        let adaptor_sig_bc: AdaptorSignature = musig2::adaptor::aggregate_partial_signatures(
            &key_agg_ctx_bc,
            &aggnonce_bc,
            adaptor_point_bc,
            partial_sigs_bc.iter().copied(),
            &message,
        )
            .expect("failed to aggregate BC adaptor signatures");

        // B and C pass adaptor_sig_bc to A
        musig2::adaptor::verify_single(
            aggregated_pubkey_bc,
            &adaptor_sig_bc,
            &message,
            adaptor_point_bc,
        )
            .expect("invalid BC adaptor signature");

        // Here must be actual spending note but we just test with signature
        let preimage: [u8; 32] = adaptor_secret_b.serialize();
        let note_message = Element::from(100u64);
        let signature = Signature32 {
            preimage,
            message: note_message,
        };
        let proof = signature.prove().unwrap();
        proof.verify().expect("Verification failed");

        // Here recipient spends from note to her address and completes backup tx
        let valid_sig_bc: musig2::LiftedSignature = adaptor_sig_bc.adapt(adaptor_secret_bc).unwrap();
        musig2::verify_single(
            aggregated_pubkey_bc,
            valid_sig_bc,
            &message,
        )
            .expect("invalid decrypted BC signature");
    }

    #[test]
    fn test_signature_input_conversion() {
        // Create a signature with a test preimage and message
        let preimage = [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
            17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
        ];
        let message = Element::from(101u64);
        let signature = Signature32 {
            preimage,
            message,
        };

        // Convert to SignatureInput
        let input = Signature32Input::from(&signature);

        // Verify the conversion
        assert_eq!(input.preimage, preimage);
        assert_eq!(input.message, message.to_base());
        assert_eq!(input.address, signature.address().to_base());
        assert_eq!(input.message_hash, signature.message_hash().to_base());
    }
}
