use common::bytes_32_to_field_pair;
use poseidon::poseidon2;
use sha256::digest;

fn main(
    message_hash: Field,
    preimage: [u8; 32],
    sha_hash: [u8; 32],
    poseidon_hash: pub Field,
    message: pub Field,
) {
    let mut computed_sha_hash = digest(preimage);
    assert(sha_hash == computed_sha_hash, "SHA does not correspond to preimage");

    let (low, high) = bytes_32_to_field_pair(computed_sha_hash);

    let computed_hash = poseidon2::Poseidon2::hash([low, high], 2);

    assert(poseidon_hash == computed_hash, "Address is not owned by the owner");

    let calc_message_hash = poseidon2::Poseidon2::hash([low, high, message], 3);
    assert(calc_message_hash == message_hash, "Message hash is invalid");
}

#[test]
fn test_main() {
    let preimage = [
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
        26, 27, 28, 29, 30, 31, 32,
    ];
    let message = 101 as Field;
    let message_hash = 0x25975cf1573a3fa981957048b6629cc5e580272d6d78f5186fd781f586be1223;
    let hash = 0x726fb5dbd1cf1623d05266d8ff07d689cd841f7f656b31163170962acd0cbfc;
    let sha = [
        174, 33, 108, 46, 245, 36, 122, 55, 130, 193, 53, 239, 162, 121, 163, 228, 205, 198, 16,
        148, 39, 15, 93, 43, 229, 140, 98, 4, 183, 166, 18, 201,
    ];

    // Call your main circuit function
    main(message_hash, preimage, sha, hash, message);
}
