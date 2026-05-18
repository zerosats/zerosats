use common::get_secret_hash;
use poseidon::poseidon2::Poseidon2;

// Lagrange interpolation at x = 0 for a degree-1 polynomial recovered from two
// distinct shares. The polynomial used by `shamir_disclosure` for 2-of-3 is
// `y = secret + c1 * x`, so the secret equals
// (y1 * x2 - y2 * x1) / (x2 - x1).
fn reconstruct_secret_2_of_3(
    share_1_x: Field,
    share_1_y: Field,
    share_2_x: Field,
    share_2_y: Field,
) -> Field {
    assert(share_1_x != share_2_x, "Shares must have distinct x-coordinates");
    (share_1_y * share_2_x - share_2_y * share_1_x) / (share_2_x - share_1_x)
}

fn main(
    share_1_x: Field,
    share_1_y: Field,
    share_2_x: Field,
    share_2_y: Field,
    message_hash: Field,
    address: pub Field,
    message: pub Field,
) {
    let owner_pk = reconstruct_secret_2_of_3(share_1_x, share_1_y, share_2_x, share_2_y);

    let owner_address = get_secret_hash(owner_pk);
    assert(owner_address == address, "Address is not owned by the owner");

    let calc_message_hash = Poseidon2::hash([owner_pk, message], 2);
    assert(calc_message_hash == message_hash, "Message hash is invalid");
}

#[test]
fn test_reconstruct_and_verify() {
    let owner_pk: Field = 0x1234567890abcdef;
    let coeff: Field = 0x99;
    let message: Field = 42;

    let share_1_x: Field = 1;
    let share_2_x: Field = 3;
    let share_1_y: Field = owner_pk + coeff * share_1_x;
    let share_2_y: Field = owner_pk + coeff * share_2_x;

    let address = get_secret_hash(owner_pk);
    let message_hash = Poseidon2::hash([owner_pk, message], 2);

    main(
        share_1_x,
        share_1_y,
        share_2_x,
        share_2_y,
        message_hash,
        address,
        message,
    );
}

#[test(should_fail_with = "Address is not owned by the owner")]
fn test_rejects_bad_address() {
    let owner_pk: Field = 7;
    let coeff: Field = 11;
    let message: Field = 1;

    let share_1_y: Field = owner_pk + coeff * 1;
    let share_2_y: Field = owner_pk + coeff * 2;

    let message_hash = Poseidon2::hash([owner_pk, message], 2);

    main(1, share_1_y, 2, share_2_y, message_hash, 0, message);
}

#[test(should_fail_with = "Message hash is invalid")]
fn test_rejects_bad_message_hash() {
    let owner_pk: Field = 7;
    let coeff: Field = 11;
    let message: Field = 1;

    let share_1_y: Field = owner_pk + coeff * 1;
    let share_2_y: Field = owner_pk + coeff * 2;

    let address = get_secret_hash(owner_pk);

    main(1, share_1_y, 2, share_2_y, 0, address, message);
}

#[test(should_fail_with = "Shares must have distinct x-coordinates")]
fn test_rejects_duplicate_x() {
    let owner_pk: Field = 7;
    let message: Field = 1;
    let address = get_secret_hash(owner_pk);
    let message_hash = Poseidon2::hash([owner_pk, message], 2);

    main(1, 18, 1, 18, message_hash, address, message);
}
