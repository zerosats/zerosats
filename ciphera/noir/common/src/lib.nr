use poseidon::poseidon2;
use bitcoin::check_pow_chain;

pub struct TimeLock {
    pub zero_block: [u8; 32],
    pub n_blocks: Field,
}

pub struct TimeProof {
    pub lock: TimeLock,
    pub headers: [[u8; 80]; 2],
}

pub struct Note {
    pub kind: Field,
    pub value: Field,
    pub address: Field,
    pub psi: Field,
}

pub struct InputNote {
    pub note: Note,
    pub secret_key: Field,
    pub preimage: [u8; 32],
    pub time_proof: TimeProof,
}

pub fn check_commitment(note: Note, commitment: Field) {
    assert(get_note_commitment(note) == commitment, "Note commitment is not valid");
}

pub fn get_note_commitment(note: Note) -> Field {
    if (note.kind == 0) {
        assert(note.value == 0, "Padding notes must be zero value");
        0
    } else {
        poseidon2::Poseidon2::hash(
            [0x2, note.kind, note.value, note.address, note.psi, 0, 0],
            7,
        )
    }
}

pub fn get_secret_hash(secret_key: Field) -> Field {
    poseidon2::Poseidon2::hash([secret_key, 0], 2)
}

pub fn get_timelock_commitment(lock: TimeLock) -> Field {
    let (low, high) = bytes_32_to_field_pair(lock.zero_block);
    let zero_block_hash = poseidon2::Poseidon2::hash([low, high], 2);
     poseidon2::Poseidon2::hash([zero_block_hash, lock.n_blocks], 2)
}

pub fn check_spend_conditions(input_note: InputNote) {
    if input_note.note.value != 0 {
        if is_one_of(input_note.note.kind, [1, 2, 3, 4]) {
            assert(
                        get_secret_hash(input_note.secret_key) == input_note.note.address,
                        "Input note is not owned by the owner",
                    );
        }
        // signature32
        if input_note.note.kind == 5 {
            let (low, high) = bytes_32_to_field_pair(input_note.preimage);

            let computed_hash = poseidon2::Poseidon2::hash([low, high], 2);
            assert(computed_hash == input_note.note.address, "Address is not owned by the owner");
        }
        // signature32sha
        if input_note.note.kind == 6 {
            let computed_sha_hash = sha256::digest(input_note.preimage);
            let (low, high) = bytes_32_to_field_pair(computed_sha_hash);

            let computed_hash = poseidon2::Poseidon2::hash([low, high], 2);
            assert(computed_hash == input_note.note.address, "Address is not owned by the owner");
        }
        // timelocked
        if input_note.note.kind == 7 {
            let key_hash = get_secret_hash(input_note.secret_key);
            let commitment = get_timelock_commitment(input_note.time_proof.lock);
            let timelocked_address = poseidon2::Poseidon2::hash([key_hash, commitment], 2);
            assert(
                        timelocked_address == input_note.note.address,
                        "Input note is not owned by the owner",
                    );
            // Not fool-proof. Not checking min difficulty to save on circuit constraints
            assert(check_pow_chain(input_note.time_proof.headers, input_note.time_proof.lock.zero_block, input_note.time_proof.lock.n_blocks));
        }
        // time- and hashlocked
        if input_note.note.kind == 8 {
            if input_note.preimage != [0; 32] {
                let computed_sha_hash = sha256::digest(input_note.preimage);
                let (low, high) = bytes_32_to_field_pair(computed_sha_hash);

                let computed_hash = poseidon2::Poseidon2::hash([low, high], 2);
                assert(computed_hash == input_note.note.psi, "HTLC: computed hash does not match to SHA preimage");
            } else {
                let key_hash = get_secret_hash(input_note.secret_key);
                let commitment = get_timelock_commitment(input_note.time_proof.lock);
                let timelocked_address = poseidon2::Poseidon2::hash([key_hash, commitment], 2);

                assert(
                            timelocked_address == input_note.note.address,
                            "HTLC: wrong refund commitment",
                        );

                // Not fool-proof. Not checking min difficulty to save on circuit constraints
                assert(
                    check_pow_chain(input_note.time_proof.headers, input_note.time_proof.lock.zero_block, input_note.time_proof.lock.n_blocks),
                    "HTLC: timelocked"
                    );
            }
        }
    }
}

pub fn slice<let N: u32, let L: u32>(bytes: [u8; N], start: u32) -> [u8; L] {
    let end = start + L;
    assert(end <= N, "slice: slice end out of bounds");
    let mut result = [bytes[0]; L];
    for i in 0..L {
        result[i] = bytes[start + i];
    }
    result
}

pub fn field_from_bytes<let N: u32>(bytes: [u8; N]) -> Field {
    assert(bytes.len() < 32, "field_from_bytes: N must be less than 32");
    let mut as_field = 0;
    let mut offset = 1;
    for i in 0..N {
        as_field += (bytes[i] as Field) * offset;
        offset *= 256;
    }
    as_field
}

pub fn is_one_of(value: Field, one_of: [Field]) -> bool {
    let mut is_one_of = false;
    for possible in one_of {
        if possible == value {
            is_one_of = true;
        }
    }
    is_one_of
}

pub fn bytes_32_to_field_pair(bytes: [u8; 32]) -> (Field, Field) {
  let mut low_bytes = [0u8; 16];
  let mut high_bytes = [0u8; 16];
  for i in 0..16 {
      low_bytes[i] = bytes[i];
      high_bytes[i] = bytes[i + 16];
  }
  (Field::from_be_bytes::<16>(low_bytes), Field::from_be_bytes::<16>(high_bytes))
}