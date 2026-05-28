use element::Element;
use flate2::{Compression, write::GzEncoder};
use std::io::Write;
use std::str::FromStr;
use zk_primitives::{
    AggAgg, AggEscrow, AggLeafSource, AggUtxo, Escrow, EscrowInputNote, EscrowProof,
    EscrowProofBundleWithMerkleProofs, InputNote, MerklePath, Note, TimeLock, TimeProof, ToBytes,
    Utxo, UtxoKind, UtxoProof, UtxoProofBundleWithMerkleProofs, get_address_for_private_key,
};

use crate::{Prove, Result, Verify};

pub fn get_keypair(key: u64) -> (Element, Element) {
    let secret_key = Element::new(key);
    let address = get_address_for_private_key(secret_key);
    (secret_key, address)
}

pub fn send_note(value: u64, address: Element, psi: Element) -> Note {
    Note::new_with_psi(address, Element::new(value), psi)
}

pub fn note(value: u64, address: Element, psi: Element, note_kind: Element) -> Note {
    Note::new_with_note_kind(address, Element::new(value), psi, note_kind)
}

pub fn compress_proof(proof: &impl ToBytes) -> Vec<u8> {
    let mut proof_gz = Vec::new();
    let mut encoder = GzEncoder::new(&mut proof_gz, Compression::best());
    encoder.write_all(proof.to_bytes().as_slice()).unwrap();
    encoder.finish().unwrap();
    proof_gz
}

pub fn prove_proof<P: Prove>(proof_input: &P, proof_type: &str) -> Result<P::Proof> {
    let start = std::time::Instant::now();
    let proof = proof_input.prove().unwrap();
    let end = std::time::Instant::now() - start;
    println!("Proving {proof_type} completed in {end:?}");
    let proof_gz = compress_proof(&proof);
    println!(
        "Proof size: {:?} (compressed: {:?})",
        proof.to_bytes().len(),
        &proof_gz.len()
    );
    Ok(proof)
}

pub fn verify_proof(proof: &impl Verify, proof_type: &str) {
    let start = std::time::Instant::now();
    let result = proof.verify();
    let duration = start.elapsed();
    // TODO: Dec 2025, three tests are failing if bb is not searched in PATH via "which", see bb_cli
    assert!(
        result.is_ok(),
        "Proof verification failed: {:?}",
        result.err()
    );

    println!("Proof {proof_type} verification completed in {duration:?}");
}

pub fn prove_and_verify<P: Prove>(proof_input: &P, proof_type: &str) -> Result<P::Proof> {
    let proof = prove_proof(proof_input, proof_type)?;
    verify_proof(&proof, proof_type);
    Ok(proof)
}

#[test]
fn test_utxo() {
    let (secret_key, address) = get_keypair(101);

    let input_note1 = InputNote::new(send_note(50, address, Element::new(1)), secret_key);
    let input_note2 = InputNote::new(send_note(30, address, Element::new(2)), secret_key);

    let output_note1 = send_note(40, address, Element::new(3));
    let output_note2 = send_note(40, address, Element::new(4));

    let utxo = Utxo::new_send(
        [input_note1, input_note2],
        [output_note1, output_note2],
    );

    prove_and_verify(&utxo, "utxo").unwrap();
}

fn process_utxo_for_agg_generic(
    tree: &mut smirk::Tree<161, ()>,
    utxo: &Utxo,
) -> Result<(
    UtxoProof,
    MerklePath<161>,
    MerklePath<161>,
    MerklePath<161>,
    MerklePath<161>,
    Element,
)> {
    let utxo_proof = utxo.prove()?;
    let padding_path = MerklePath::default();

    let p1 = if utxo.input_notes[0].note.commitment().is_zero() {
        padding_path.clone()
    } else {
        let path = MerklePath::new(
            tree.path_for(utxo.input_notes[0].note.commitment())
                .siblings
                .to_vec(),
        );
        tree.remove(utxo.input_notes[0].note.commitment()).unwrap();
        path
    };

    let p2 = if utxo.input_notes[1].note.commitment().is_zero() {
        padding_path.clone()
    } else {
        let path = MerklePath::new(
            tree.path_for(utxo.input_notes[1].note.commitment())
                .siblings
                .to_vec(),
        );
        tree.remove(utxo.input_notes[1].note.commitment()).unwrap();
        path
    };

    let p3 = if utxo.output_notes[0].commitment().is_zero() {
        padding_path.clone()
    } else {
        tree.insert(utxo.output_notes[0].commitment(), ()).unwrap();
        MerklePath::new(
            tree.path_for(utxo.output_notes[0].commitment())
                .siblings
                .to_vec(),
        )
    };

    let p4 = if utxo.output_notes[1].commitment().is_zero() {
        padding_path
    } else {
        tree.insert(utxo.output_notes[1].commitment(), ()).unwrap();
        MerklePath::new(
            tree.path_for(utxo.output_notes[1].commitment())
                .siblings
                .to_vec(),
        )
    };

    let new_root = tree.root_hash();
    Ok((utxo_proof, p1, p2, p3, p4, new_root))
}

fn process_utxo_for_agg(
    tree: &mut smirk::Tree<161, ()>,
    utxo: &Utxo,
) -> Result<(
    UtxoProof,
    MerklePath<161>,
    MerklePath<161>,
    MerklePath<161>,
    MerklePath<161>,
    Element,
)> {
    let utxo_proof = utxo.prove()?;

    let p1: MerklePath<161> = MerklePath::new(
        tree.path_for(utxo.input_notes[0].note.commitment())
            .siblings
            .to_vec(),
    );
    tree.remove(utxo.input_notes[0].note.commitment()).unwrap();

    let p2: MerklePath<161> = MerklePath::new(
        tree.path_for(utxo.input_notes[1].note.commitment())
            .siblings
            .to_vec(),
    );
    tree.remove(utxo.input_notes[1].note.commitment()).unwrap();

    tree.insert(utxo.output_notes[0].commitment(), ()).unwrap();
    let p3: MerklePath<161> = MerklePath::new(
        tree.path_for(utxo.output_notes[0].commitment())
            .siblings
            .to_vec(),
    );

    tree.insert(utxo.output_notes[1].commitment(), ()).unwrap();
    let p4: MerklePath<161> = MerklePath::new(
        tree.path_for(utxo.output_notes[1].commitment())
            .siblings
            .to_vec(),
    );

    let new_root = tree.root_hash();

    Ok((utxo_proof, p1, p2, p3, p4, new_root))
}

#[test]
fn test_agg_utxo() {
    let (secret_key, address) = get_keypair(101);
    let mut tree = smirk::Tree::<161, ()>::new();

    let utxo1_input_note1 = InputNote {
        note: send_note(50, address, Element::new(1)),
        secret_key,
        ..InputNote::default()
    };
    tree.insert(utxo1_input_note1.note.commitment(), ())
        .unwrap();

    let utxo1_input_note2 = InputNote {
        note: send_note(30, address, Element::new(2)),
        secret_key,
        ..InputNote::default()
    };
    tree.insert(utxo1_input_note2.note.commitment(), ())
        .unwrap();

    let utxo1_old_root = tree.root_hash();

    let utxo1_output_note1 = send_note(40, address, Element::new(3));
    let utxo1_output_note2 = send_note(40, address, Element::new(4));

    let utxo1 = Utxo {
        input_notes: [utxo1_input_note1.clone(), utxo1_input_note2.clone()],
        output_notes: [utxo1_output_note1.clone(), utxo1_output_note2.clone()],
        kind: UtxoKind::Send,
        burn_address: None,
    };

    let (utxo1_proof, p1, p2, p3, p4, utxo1_new_root) =
        process_utxo_for_agg(&mut tree, &utxo1).unwrap();
    verify_proof(&utxo1_proof, "utxo");

    let agg_utxo1 = AggUtxo::new(
        [
            UtxoProofBundleWithMerkleProofs::new(utxo1_proof, &[p1, p2, p3, p4]),
            UtxoProofBundleWithMerkleProofs::default(),
            UtxoProofBundleWithMerkleProofs::default(),
        ],
        utxo1_old_root,
        utxo1_new_root,
    );

    prove_and_verify(&agg_utxo1, "agg_utxo").unwrap();
}

#[test]
fn test_agg_agg() {
    let (secret_key, address) = get_keypair(101);
    let mut tree = smirk::Tree::<161, ()>::new();

    let utxo1_input_note1 = InputNote {
        note: send_note(60, address, Element::new(1)),
        secret_key,
        ..InputNote::default()
    };
    tree.insert(utxo1_input_note1.note.commitment(), ())
        .unwrap();

    let utxo1_input_note2 = InputNote {
        note: send_note(40, address, Element::new(2)),
        secret_key,
        ..InputNote::default()
    };
    tree.insert(utxo1_input_note2.note.commitment(), ())
        .unwrap();

    let utxo1_old_root = tree.root_hash();

    let utxo1_output_note1 = send_note(70, address, Element::new(3));
    let utxo1_output_note2 = send_note(30, address, Element::new(4));

    let utxo1 = Utxo {
        input_notes: [utxo1_input_note1.clone(), utxo1_input_note2.clone()],
        output_notes: [utxo1_output_note1.clone(), utxo1_output_note2.clone()],
        kind: UtxoKind::Send,
        burn_address: None,
    };

    let (utxo1_proof, p1_1, p1_2, p1_3, p1_4, utxo1_new_root) =
        process_utxo_for_agg(&mut tree, &utxo1).unwrap();
    verify_proof(&utxo1_proof, "utxo");

    let agg_utxo1 = AggUtxo::new(
        [
            UtxoProofBundleWithMerkleProofs::new(utxo1_proof, &[p1_1, p1_2, p1_3, p1_4]),
            UtxoProofBundleWithMerkleProofs::default(),
            UtxoProofBundleWithMerkleProofs::default(),
        ],
        utxo1_old_root,
        utxo1_new_root,
    );
    let agg_utxo1_proof = prove_and_verify(&agg_utxo1, "agg_utxo").unwrap();

    let utxo2_input_note1 = InputNote {
        note: utxo1_output_note1.clone(),
        secret_key,
        ..InputNote::default()
    };

    let utxo2_input_note2 = InputNote {
        note: utxo1_output_note2.clone(),
        secret_key,
        ..InputNote::default()
    };

    let utxo2_old_root = tree.root_hash();

    let utxo2_output_note1 = send_note(55, address, Element::new(5));
    let utxo2_output_note2 = send_note(45, address, Element::new(6));

    let utxo2 = Utxo {
        input_notes: [utxo2_input_note1.clone(), utxo2_input_note2.clone()],
        output_notes: [utxo2_output_note1.clone(), utxo2_output_note2.clone()],
        kind: UtxoKind::Send,
        burn_address: None,
    };

    let (utxo2_proof, p2_1, p2_2, p2_3, p2_4, utxo2_new_root) =
        process_utxo_for_agg(&mut tree, &utxo2).unwrap();
    verify_proof(&utxo2_proof, "utxo");

    let agg_utxo2 = AggUtxo::new(
        [
            UtxoProofBundleWithMerkleProofs::new(utxo2_proof, &[p2_1, p2_2, p2_3, p2_4]),
            UtxoProofBundleWithMerkleProofs::default(),
            UtxoProofBundleWithMerkleProofs::default(),
        ],
        utxo2_old_root,
        utxo2_new_root,
    );
    let agg_utxo2_proof = prove_and_verify(&agg_utxo2, "agg_utxo").unwrap();

    let agg_agg = AggAgg::new([agg_utxo1_proof, agg_utxo2_proof]);

    prove_and_verify(&agg_agg, "agg_agg").unwrap();
}

#[test]
fn test_alt_agg_utxo() {
    let (secret_key, address) = get_keypair(202);
    let mut tree = smirk::Tree::<161, ()>::new();

    // Pre-insert SlowBurn input notes so they exist before the aggregate's old_root
    let slow_input_note1 = InputNote {
        note: send_note(25, address, Element::new(12)),
        secret_key,
        ..InputNote::default()
    };
    tree.insert(slow_input_note1.note.commitment(), ()).unwrap();

    let slow_input_note2 = InputNote {
        note: send_note(15, address, Element::new(13)),
        secret_key,
        ..InputNote::default()
    };
    tree.insert(slow_input_note2.note.commitment(), ()).unwrap();

    let old_root = tree.root_hash();

    // --- UTXO 1: Mint (padding inputs, 2 real outputs) ---
    let mint_output_note1 = send_note(30, address, Element::new(10));
    let mint_output_note2 = send_note(20, address, Element::new(11));
    let mint_utxo = Utxo::new_mint([mint_output_note1.clone(), mint_output_note2.clone()]);

    let (mint_proof, mp1, mp2, mp3, mp4, _) =
        process_utxo_for_agg_generic(&mut tree, &mint_utxo).unwrap();
    verify_proof(&mint_proof, "utxo");

    // --- UTXO 2: Burn (mint outputs as inputs, padding outputs) ---
    let burn_address = Element::new(0xDeadBeef);
    let burn_utxo = Utxo::new_burn(
        [
            InputNote {
                note: mint_output_note1.clone(),
                secret_key,
                ..InputNote::default()
            },
            InputNote {
                note: mint_output_note2.clone(),
                secret_key,
                ..InputNote::default()
            },
        ],
        burn_address,
    );

    let (burn_proof, bp1, bp2, bp3, bp4, _) =
        process_utxo_for_agg_generic(&mut tree, &burn_utxo).unwrap();
    verify_proof(&burn_proof, "utxo");

    // --- UTXO 3: SlowBurn (pre-inserted inputs, padding outputs) ---
    let slow_utxo = Utxo::new_burn_no_sub(
        [slow_input_note1.clone(), slow_input_note2.clone()],
        burn_address,
    );

    let (slow_proof, np1, np2, np3, np4, new_root) =
        process_utxo_for_agg_generic(&mut tree, &slow_utxo).unwrap();
    verify_proof(&slow_proof, "utxo");

    let agg_utxo = AggUtxo::new(
        [
            UtxoProofBundleWithMerkleProofs::new(mint_proof, &[mp1, mp2, mp3, mp4]),
            UtxoProofBundleWithMerkleProofs::new(burn_proof, &[bp1, bp2, bp3, bp4]),
            UtxoProofBundleWithMerkleProofs::new(slow_proof, &[np1, np2, np3, np4]),
        ],
        old_root,
        new_root,
    );

    prove_and_verify(&agg_utxo, "agg_utxo").unwrap();
}

// =====================================================================
// Spend-path tests for note kinds 5 (signature32), 6 (signature32sha),
// 7 (timelock), and 8 (HTLC: SHA preimage path + timelock refund path).
//
// In Rust terms, the Noir circuit's `note.utxo_kind` field maps to
// `zk_primitives::note.note_kind` (see `barretenberg::circuits::note::BNote::from`),
// so we set `contract` to 5/6/7/8 to select the spend path.
//
// The PoW chain fixture below reuses the exact bytes from
// noir/timelock/src/main.nr (block 946920 + headers 946921 & 946922),
// which is the same data that drives the Noir `test_main_two_blocks` test.
// =====================================================================

use sha2::{Digest, Sha256};

fn shared_preimage_bytes() -> [u8; 32] {
    [
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
        26, 27, 28, 29, 30, 31, 32,
    ]
}

// Poseidon of the 32-byte preimage's (high, low) 16-byte halves. Matches
// `signature32` and the Noir kind-5 ownership check.
fn signature32_address(preimage: [u8; 32]) -> Element {
    let element = Element::from_be_bytes(preimage);
    let (high, low) = element.decompose_be();
    hash::hash_merge([high, low])
}

// Poseidon of the SHA-256 digest's (high, low) 16-byte halves. Matches
// `signature32sha` and the Noir kind-6 ownership check; also the encoding
// of `note.psi` for the kind-8 hash-path.
fn signature32sha_address(secret_opt: Option<Element>, preimage: [u8; 32]) -> Element {
    let sha: [u8; 32] = Sha256::digest(preimage).into();
    let element = Element::from_be_bytes(sha);
    let (high, low) = element.decompose_be();

    match secret_opt {
        Some(secret_key) => {
            let key_hash = get_address_for_private_key(secret_key);
            hash::hash_merge([key_hash, high, low])
        }
        None => {
            hash::hash_merge([high, low])
        }
    }
}

fn timelock_commitment(lock: &TimeLock) -> Element {
    let element = Element::from_be_bytes(lock.zero_block);
    let (high, low) = element.decompose_be();
    let zero_block_hash = hash::hash_merge([high, low]);
    hash::hash_merge([zero_block_hash, lock.n_blocks])
}

// Address for the timelock-locked / HTLC-refund spend path:
//   Poseidon(get_secret_hash(sk), timelock_commitment).
// `get_secret_hash` is Poseidon([sk, 0]) which is what
// `get_address_for_private_key` already does.
fn timelock_address(secret_key: Element, lock: &TimeLock) -> Element {
    let key_hash = get_address_for_private_key(secret_key);
    let commitment = timelock_commitment(&lock);
    hash::hash_merge([key_hash, commitment])
}

// Block 946920 hash (LE) -- the anchor for the PoW chain fixture.
fn anchor_zero_block() -> [u8; 32] {
    [
        0xf8, 0xa1, 0x7c, 0xed, 0x1d, 0xac, 0x17, 0xba, 0x27, 0xba, 0x9d, 0xee, 0x7f, 0x63, 0x95,
        0x9b, 0xa7, 0x54, 0x18, 0xb6, 0x7c, 0xe7, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ]
}

// 80-byte serialized header for block 946921, prev_hash linking to 946920.
fn header_946921() -> [u8; 80] {
    [
        0x00, 0x40, 0x0b, 0x20, 0xf8, 0xa1, 0x7c, 0xed, 0x1d, 0xac, 0x17, 0xba, 0x27, 0xba, 0x9d,
        0xee, 0x7f, 0x63, 0x95, 0x9b, 0xa7, 0x54, 0x18, 0xb6, 0x7c, 0xe7, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xee, 0xa2, 0x39, 0xdc, 0xe3, 0x77, 0x3c, 0x5f, 0x61,
        0x79, 0xd2, 0xd1, 0x49, 0xb2, 0x5f, 0x1b, 0x17, 0xf6, 0x49, 0x33, 0x86, 0x95, 0x5c, 0xf5,
        0x3f, 0xc7, 0x04, 0x5a, 0x39, 0xb8, 0xc6, 0x00, 0x0c, 0xc8, 0xef, 0x69, 0x69, 0x13, 0x02,
        0x17, 0xe3, 0x10, 0xa9, 0x35,
    ]
}

// 80-byte serialized header for block 946922.
fn header_946922() -> [u8; 80] {
    [
        0x00, 0x00, 0x07, 0x20, 0xcf, 0x51, 0x90, 0x4c, 0xcc, 0x0c, 0xf4, 0x7b, 0x6a, 0xab, 0xf0,
        0xcc, 0xfe, 0x55, 0x5c, 0x19, 0x77, 0x7c, 0xf6, 0x62, 0x06, 0x01, 0x02, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x74, 0x3f, 0xc7, 0xf1, 0xaf, 0xc9, 0x8f, 0x0e, 0x2f,
        0x4e, 0x20, 0xc4, 0x0c, 0xb2, 0x11, 0x35, 0x07, 0x8a, 0x30, 0x5c, 0x01, 0xb9, 0x05, 0xe7,
        0xc5, 0x26, 0xac, 0x10, 0xb7, 0xb4, 0x25, 0xc9, 0xf2, 0xc9, 0xef, 0x69, 0x69, 0x13, 0x02,
        0x17, 0x65, 0xdb, 0x8d, 0x21,
    ]
}

// Lock + 2-header PoW witness corresponding to the anchor / header fixtures.
// This is the same chain proved by `noir/timelock::test_main_two_blocks`.
fn pow_two_block_proof() -> TimeProof {
    TimeProof {
        lock: TimeLock {
            zero_block: anchor_zero_block(),
            n_blocks: Element::new(2),
        },
        headers: [header_946921(), header_946922()],
    }
}

fn pow_two_block_lock() -> TimeLock {
    TimeLock {
        zero_block: anchor_zero_block(),
        n_blocks: Element::new(2),
    }
}

#[test]
fn test_escrow_signature32_spend() {
    // Kind 5: ownership proven by a 32-byte preimage whose Poseidon
    // hash (over its high/low 16-byte halves), Equals `note.address`.
    let preimage = shared_preimage_bytes();
    let address = signature32_address(preimage);
    let note_kind = Element::new(1);

    let input_note = EscrowInputNote {
        note: note(50, address, Element::new(1), note_kind),
        spend_type: 1,
        secret_key: Element::ZERO,
        preimage,
        ..EscrowInputNote::default()
    };

    let escrow = Escrow {
        kind: UtxoKind::Send,
        input_notes: [input_note, EscrowInputNote::padding_note()],
        output_notes: [
            note(30, Element::new(42), Element::new(3), note_kind),
            note(20, Element::new(43), Element::new(4), note_kind),
        ],
        burn_address: None,
    };

    prove_and_verify(&escrow, "escrow").unwrap();
}

#[test]
fn test_escrow_signature32sha_spend() {
    // Kind 6: ownership proven by revealing a SHA-256 preimage whose
    // digest hashes (under Poseidon, over high/low halves) to `note.address`.
    let preimage = shared_preimage_bytes();
    let address = signature32sha_address(None, preimage);
    let kind6 = Element::new(6);

    let input_note = EscrowInputNote {
        note: note(40, address, Element::new(1), kind6),
        spend_type: 2,
        secret_key: Element::ZERO,
        preimage,
        ..EscrowInputNote::default()
    };

    let escrow = Escrow {
        kind: UtxoKind::Send,
        input_notes: [input_note, EscrowInputNote::padding_note()],
        output_notes: [
            note(25, Element::new(99), Element::new(3), kind6),
            note(15, Element::new(100), Element::new(4), kind6),
        ],
        burn_address: None,
    };

    prove_and_verify(&escrow,"escrow").unwrap();
}

#[test]
fn test_escrow_normal_timelocked_spend() {
    let (secret_key, normal_address) = get_keypair(101);

    assert_eq!(normal_address, Element::from_str("0x22d68b303a6a3d416959fb363795548966049a655418ebd9eb6a818fd6d2e27b").unwrap());

    let lock = pow_two_block_lock();
    let locked_address = timelock_address(secret_key, &lock);
    let note_kind = Element::new(1);

    let input_note = EscrowInputNote {
        note: note(50, normal_address, locked_address, note_kind),
        spend_type: 0,
        secret_key,
        preimage: [0u8; 32],
        time_proof: pow_two_block_proof(),
    };

    let escrow = Escrow {
        kind: UtxoKind::Send,
        input_notes: [input_note, EscrowInputNote::padding_note()],
        output_notes: [
            note(20, Element::new(7), Element::new(3), note_kind),
            note(30, Element::new(8), Element::new(4), note_kind),
        ],
        burn_address: None,
    };

    prove_and_verify(&escrow, "escrow").unwrap();
}

#[test]
fn test_escrow_sha_htlc_hash_path() {
    let (secret_key, refund_address) = get_keypair(202);

    let preimage = shared_preimage_bytes();
    let sha_address = signature32sha_address(Some(secret_key), preimage);
    let note_kind = Element::new(8);

    // The address here is irrelevant for the hash path; pick something
    // simple. The circuit only constrains `note.psi`.
    let input_note = EscrowInputNote {
        note: Note {
            utxo_kind: Element::new(2),
            note_kind,
            address: sha_address,
            psi: refund_address,
            value: Element::new(50),
        },
        spend_type: 3,
        secret_key,
        preimage,
        time_proof: pow_two_block_proof(),
    };

    let escrow = Escrow {
        kind: UtxoKind::Send,
        input_notes: [input_note, EscrowInputNote::padding_note()],
        output_notes: [
            note(20, Element::new(50), Element::new(3), note_kind),
            note(30, Element::new(51), Element::new(4), note_kind),
        ],
        burn_address: None,
    };

    prove_and_verify(&escrow, "escrow").unwrap();
}

#[test]
fn test_escrow_sha_htlc_refund_path() {
    let (secret_key, refund_address) = get_keypair(202);
    let lock = pow_two_block_lock();
    let timelocked_address = timelock_address(secret_key, &lock);
    let note_kind = Element::new(1);

    let input_note = EscrowInputNote {
        note: Note {
            utxo_kind: Element::new(2),
            note_kind,
            address: refund_address,
            psi: timelocked_address, // unconstrained on this path
            value: Element::new(60),
        },
        spend_type: 3,
        secret_key,
        preimage: [0u8; 32],
        time_proof: pow_two_block_proof(),
    };

    let escrow = Escrow {
        kind: UtxoKind::Send,
        input_notes: [input_note, EscrowInputNote::padding_note()],
        output_notes: [
            note(25, Element::new(60), Element::new(3), note_kind),
            note(35, Element::new(61), Element::new(4), note_kind),
        ],
        burn_address: None,
    };

    prove_and_verify(&escrow, "escrow").unwrap();
}

// Same shape as `process_utxo_for_agg` but takes an `Escrow` and
// returns an `EscrowProof`. Removes input commitments / inserts output
// commitments and emits the four Merkle paths in
// (input0, input1, output0, output1) order.
fn process_escrow_for_agg(
    tree: &mut smirk::Tree<161, ()>,
    escrow: &Escrow,
) -> Result<(
    EscrowProof,
    MerklePath<161>,
    MerklePath<161>,
    MerklePath<161>,
    MerklePath<161>,
    Element,
)> {
    let escrow_proof = escrow.prove()?;

    let p1: MerklePath<161> = MerklePath::new(
        tree.path_for(escrow.input_notes[0].note.commitment())
            .siblings
            .to_vec(),
    );
    tree.remove(escrow.input_notes[0].note.commitment())
        .unwrap();

    let p2: MerklePath<161> = MerklePath::new(
        tree.path_for(escrow.input_notes[1].note.commitment())
            .siblings
            .to_vec(),
    );
    tree.remove(escrow.input_notes[1].note.commitment())
        .unwrap();

    tree.insert(escrow.output_notes[0].commitment(), ()).unwrap();
    let p3: MerklePath<161> = MerklePath::new(
        tree.path_for(escrow.output_notes[0].commitment())
            .siblings
            .to_vec(),
    );

    tree.insert(escrow.output_notes[1].commitment(), ()).unwrap();
    let p4: MerklePath<161> = MerklePath::new(
        tree.path_for(escrow.output_notes[1].commitment())
            .siblings
            .to_vec(),
    );

    let new_root = tree.root_hash();

    Ok((escrow_proof, p1, p2, p3, p4, new_root))
}

// End-to-end coverage for the heterogeneous-batch path:
//
//   utxo Send   -> agg_utxo   (slot 0 of agg_agg, source = AggUtxo)
//   escrow Send -> agg_escrow (slot 1 of agg_agg, source = AggEscrow)
//
// The same Merkle tree threads through both flows: utxo consumes its
// poseidon-key inputs first (old_root -> mid_root), then escrow
// consumes its signature32 inputs starting from mid_root and emits
// new_root. `agg_agg` then verifies both 1-level proofs against their
// respective verification keys via per-slot `sources`.
#[test]
fn test_agg_agg_utxo_and_escrow_mixed() {
    let (secret_key, address) = get_keypair(101);
    let mut tree = smirk::Tree::<161, ()>::new();

    // Stage 1: utxo input notes (poseidon-key path).
    let utxo_input_note1 = InputNote {
        note: send_note(60, address, Element::new(1)),
        secret_key,
        ..InputNote::default()
    };
    tree.insert(utxo_input_note1.note.commitment(), ()).unwrap();

    let utxo_input_note2 = InputNote {
        note: send_note(40, address, Element::new(2)),
        secret_key,
        ..InputNote::default()
    };
    tree.insert(utxo_input_note2.note.commitment(), ()).unwrap();

    // Stage 2: pre-insert the escrow input notes (signature32 path).
    // They have to exist in `old_root` because escrow only supports
    // Send today; there is no mint path that could conjure them up.
    let preimage = shared_preimage_bytes();
    let sig32_address = signature32_address(preimage);
    let escrow_note_kind = Element::new(1);

    let escrow_input_note1 = EscrowInputNote {
        note: note(35, sig32_address, Element::new(10), escrow_note_kind),
        spend_type: 1,
        secret_key: Element::ZERO,
        preimage,
        ..EscrowInputNote::default()
    };
    tree.insert(escrow_input_note1.note.commitment(), ())
        .unwrap();

    let escrow_input_note2 = EscrowInputNote {
        note: note(15, sig32_address, Element::new(11), escrow_note_kind),
        spend_type: 1,
        secret_key: Element::ZERO,
        preimage,
        ..EscrowInputNote::default()
    };
    tree.insert(escrow_input_note2.note.commitment(), ())
        .unwrap();

    let old_root = tree.root_hash();

    // Stage 3: utxo Send (60 + 40 = 70 + 30).
    let utxo_output_note1 = send_note(70, address, Element::new(3));
    let utxo_output_note2 = send_note(30, address, Element::new(4));

    let utxo = Utxo {
        input_notes: [utxo_input_note1.clone(), utxo_input_note2.clone()],
        output_notes: [utxo_output_note1.clone(), utxo_output_note2.clone()],
        kind: UtxoKind::Send,
        burn_address: None,
    };

    let (utxo_proof, up1, up2, up3, up4, mid_root) =
        process_utxo_for_agg(&mut tree, &utxo).unwrap();
    verify_proof(&utxo_proof, "utxo");

    let agg_utxo = AggUtxo::new(
        [
            UtxoProofBundleWithMerkleProofs::new(utxo_proof, &[up1, up2, up3, up4]),
            UtxoProofBundleWithMerkleProofs::default(),
            UtxoProofBundleWithMerkleProofs::default(),
        ],
        old_root,
        mid_root,
    );
    let agg_utxo_proof = prove_and_verify(&agg_utxo, "agg_utxo").unwrap();

    // Stage 4: escrow Send (35 + 15 = 30 + 20). Spend the two
    // signature32-locked notes that were planted in old_root.
    let escrow_output_note1 = note(30, address, Element::new(5), escrow_note_kind);
    let escrow_output_note2 = note(20, address, Element::new(6), escrow_note_kind);

    let escrow = Escrow {
        kind: UtxoKind::Send,
        input_notes: [escrow_input_note1.clone(), escrow_input_note2.clone()],
        output_notes: [escrow_output_note1.clone(), escrow_output_note2.clone()],
        burn_address: None,
    };

    let (escrow_proof, ep1, ep2, ep3, ep4, new_root) =
        process_escrow_for_agg(&mut tree, &escrow).unwrap();
    verify_proof(&escrow_proof, "escrow");

    let agg_escrow = AggEscrow::new(
        [
            EscrowProofBundleWithMerkleProofs::new(escrow_proof, &[ep1, ep2, ep3, ep4]),
            EscrowProofBundleWithMerkleProofs::default(),
            EscrowProofBundleWithMerkleProofs::default(),
        ],
        mid_root,
        new_root,
    );
    let agg_escrow_proof = prove_and_verify(&agg_escrow, "agg_escrow").unwrap();

    // Stage 5: top-level agg_agg with heterogeneous sources. The
    // `AggEscrowProof` newtype exists so `prove_and_verify` dispatches
    // to the agg_escrow verification key above; unwrap it here because
    // `AggAgg` already tracks per-slot provenance via `sources`.
    let agg_agg = AggAgg::new_mixed(
        [agg_utxo_proof, agg_escrow_proof.into_inner()],
        [AggLeafSource::AggUtxo, AggLeafSource::AggEscrow],
    );
    prove_and_verify(&agg_agg, "agg_agg_mixed").unwrap();
}
