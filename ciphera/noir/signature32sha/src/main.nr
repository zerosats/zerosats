use poseidon::poseidon2;
use sha256::digest;

fn main(message_hash: Field, preimage: [u8; 32], sha_hash: pub [u8; 32], poseidon_hash: pub Field, message: pub Field) {
    let mut computed_sha_hash = digest(preimage);
    assert(sha_hash == computed_sha_hash, "Address is not owned by the owner");

    let mut share_1_bytes = [0u8; 16];
    let mut share_2_bytes = [0u8; 16];

    for i in 0..16 {
        share_1_bytes[i] = preimage[i];
        share_2_bytes[i] = preimage[i + 16];
    }

    let share_1 = Field::from_be_bytes::<16>(share_1_bytes);
    let share_2 = Field::from_be_bytes::<16>(share_2_bytes);

    let computed_hash = poseidon2::Poseidon2::hash([share_1, share_2], 2);
    assert(poseidon_hash == computed_hash, "Address is not owned by the owner");

    let calc_message_hash = poseidon2::Poseidon2::hash([share_1, share_2, message], 3);
    assert(calc_message_hash == message_hash, "Message hash is invalid");
}

#[test]
fn test_main() {
    let preimage = [
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
        17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
    ];
    let message = 101 as Field;
    let message_hash = 0x250be1b77afbb34af96bdaecb73b4607ad2796af82c8038bb72db07fc150e883;
    let hash = 0x23eb6753772cc330ae660603827caef9c82c64f807e25b7b69a3cc1274b64b82;
    let sha = [174, 33, 108, 46, 245, 36, 122, 55, 130, 193, 53, 239, 162, 121, 163, 228, 205, 198, 16, 148, 39, 15, 93, 43, 229, 140, 98, 4, 183, 166, 18, 201];

    // Call your main circuit function
    main(message_hash, preimage, sha, hash, message);
}