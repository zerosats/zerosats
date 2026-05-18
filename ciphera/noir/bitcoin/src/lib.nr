use sha256::digest;

global MAX_CHAIN_LEN: u32 = 2;
global MIN_ALLTIME_DIFF: u32 = 35;
global DIFFICULTY_DELTA: u32 = 3;

pub fn count_leading_zero_bits(hash_le: [u8; 32]) -> u32 {
    // Reverse to big-endian
    let mut hash_be: [u8; 32] = [0; 32];
    for i in 0..32 {
        hash_be[i] = hash_le[31 - i];
    }

    let mut zeros: u32 = 0;
    let mut done:  bool = false;

    for i in 0..32 {
        let byte = hash_be[i];
        if !done {
            if byte == 0 {
                zeros = zeros + 8;
            } else {
                if byte < 0x80 { zeros = zeros + 1; }
                if byte < 0x40 { zeros = zeros + 1; }
                if byte < 0x20 { zeros = zeros + 1; }
                if byte < 0x10 { zeros = zeros + 1; }
                if byte < 0x08 { zeros = zeros + 1; }
                if byte < 0x04 { zeros = zeros + 1; }
                if byte < 0x02 { zeros = zeros + 1; }
                // byte is non-zero so it has at least one '1'
                done = true;
            }
        }
    }
    zeros
}

fn difficulty_within_bounds(anchor_zeros: u32, block_zeros: u32) -> bool {
    let lower = if anchor_zeros >= DIFFICULTY_DELTA { anchor_zeros - DIFFICULTY_DELTA } else { 0 };
    let upper = anchor_zeros + DIFFICULTY_DELTA;
    (block_zeros >= lower) & (block_zeros <= upper)
}

fn double_sha256_header(header: [u8; 80]) -> [u8; 32] {
    let h1 = digest(header);
    digest(h1)
}

fn extract_prev_hash(header: [u8; 80]) -> [u8; 32] {
    let mut out: [u8; 32] = [0; 32];
    for i in 0..32 {
        out[i] = header[4 + i];
    }
    out
}

pub fn check_pow_chain(
    headers: [[u8; 80]; MAX_CHAIN_LEN],
    anchor_hash: [u8; 32],
    n_blocks: Field,
) -> bool {
    // Range-check n_blocks before casting so a huge Field cannot truncate
    // down to a small u32 and bypass the chain-length bound.
    n_blocks.assert_max_bit_size::<32>();
    assert((n_blocks as u32) <= MAX_CHAIN_LEN, "n_blocks exceeds MAX_CHAIN_LEN");

    let anchor_zeros = count_leading_zero_bits(anchor_hash);

    let mut ok: bool = true;
    let mut active: bool = true;
    let mut expected_prev: [u8; 32] = anchor_hash;

    for i in 0..MAX_CHAIN_LEN {
        if (i as Field) == n_blocks { active = false; }

        let header = headers[i];

        println("Prev header:");
        println(extract_prev_hash(header));
        println("Expected:");
        println(expected_prev);

        // (a) chain of hashes
        let chainhash_ok = extract_prev_hash(header) == expected_prev;
        if active & !chainhash_ok { ok = false; }

        // (b) PoW difficulty
        let h = double_sha256_header(header);
        let block_zeros = count_leading_zero_bits(h);
        let diff_ok = difficulty_within_bounds(anchor_zeros, block_zeros);
        if active & !diff_ok { ok = false; }

        if active { expected_prev = h; }
    }

    ok
}