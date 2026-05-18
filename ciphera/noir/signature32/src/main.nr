use common::bytes_32_to_field_pair;
use poseidon::poseidon2;

fn main(preimage: [u8; 32], message_hash: Field, address: pub Field, message: pub Field) {
    let (low, high) = bytes_32_to_field_pair(preimage);

    let computed_hash = poseidon2::Poseidon2::hash([low, high], 2);
    assert(address == computed_hash, "Address is not owned by the owner");

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
    let message_hash = 0x250be1b77afbb34af96bdaecb73b4607ad2796af82c8038bb72db07fc150e883;
    let hash = 0x23eb6753772cc330ae660603827caef9c82c64f807e25b7b69a3cc1274b64b82;

    // Call your main circuit function
    main(preimage, message_hash, hash, message);
}
