use common::get_secret_hash;
use signature_shamir::reconstruct_secret_2_of_3;

// Proof of share membership in a 2-of-3 Shamir secret sharing scheme.
//
// Private witness: all three (x, y) shares of the scheme.
// Public inputs:   the Poseidon commitment to the SSS secret (`address`),
//                  the scheme parameters (`total`, `threshold`), and one
//                  share `(provided_x, provided_y)` whose membership is
//                  being proven.
//
// The proof convinces a verifier of three facts:
//   1. There exists a degree-(threshold-1) polynomial passing through all
//      three private shares, so the three shares are mutually consistent.
//   2. The constant term of that polynomial -- the SSS secret -- hashes
//      under `get_secret_hash` to `address`.
//   3. The public `(provided_x, provided_y)` equals one of the three
//      private shares.
fn main(
    share_x: [Field; 3],
    share_y: [Field; 3],
    address: pub Field,
    total: pub Field,
    threshold: pub Field,
    provided_x: pub Field,
    provided_y: pub Field,
) {
    // Pin scheme parameters: this circuit only proves 2-of-3 schemes.
    assert(total == 3, "verifyShare: only total = 3 is supported");
    assert(threshold == 2, "verifyShare: only threshold = 2 is supported");

    // x-coordinates must be pairwise distinct, otherwise Lagrange
    // reconstruction is undefined (and a polynomial of degree 1 can't
    // pass through "two shares" at the same x with different y).
    assert(share_x[0] != share_x[1], "Shares 0 and 1 must have distinct x");
    assert(share_x[0] != share_x[2], "Shares 0 and 2 must have distinct x");
    assert(share_x[1] != share_x[2], "Shares 1 and 2 must have distinct x");

    // Reconstruct the secret from two disjoint pairs of shares. Both
    // pairs must yield the same constant term -- this enforces that all
    // three shares lie on a single degree-1 polynomial.
    let secret_01 = reconstruct_secret_2_of_3(
        share_x[0],
        share_y[0],
        share_x[1],
        share_y[1],
    );
    let secret_02 = reconstruct_secret_2_of_3(
        share_x[0],
        share_y[0],
        share_x[2],
        share_y[2],
    );
    assert(
        secret_01 == secret_02,
        "Shares are not consistent with a degree-1 polynomial",
    );

    // The Poseidon commitment of the reconstructed secret must match
    // the public address.
    let computed_address = get_secret_hash(secret_01);
    assert(computed_address == address, "Address is not the commitment of the secret");

    // The public share must equal one of the three private shares
    // (both x and y must match -- a y that happens to lie on the line
    // at a wrong x is not a real share of this scheme).
    let matches_0 = (provided_x == share_x[0]) & (provided_y == share_y[0]);
    let matches_1 = (provided_x == share_x[1]) & (provided_y == share_y[1]);
    let matches_2 = (provided_x == share_x[2]) & (provided_y == share_y[2]);
    assert(
        matches_0 | matches_1 | matches_2,
        "Provided share is not part of this SSS scheme",
    );
}

// ---------------------------------------------------------------------
// Test fixtures
// ---------------------------------------------------------------------
//
// Build a 2-of-3 scheme `y = secret + coeff * x` and return shares at
// the three given x-coordinates.

fn shares_for(secret: Field, coeff: Field) -> ([Field; 3], [Field; 3]) {
    let xs: [Field; 3] = [1, 2, 3];
    let ys: [Field; 3] = [
        secret + coeff * xs[0],
        secret + coeff * xs[1],
        secret + coeff * xs[2],
    ];
    (xs, ys)
}

#[test]
fn test_verify_share_happy_path() {
    let secret: Field = 0x1234567890abcdef;
    let coeff: Field = 0x99;
    let (xs, ys) = shares_for(secret, coeff);
    let address = get_secret_hash(secret);

    // Prove that share #1 (index 1) is part of the scheme.
    main(xs, ys, address, 3, 2, xs[1], ys[1]);
}

#[test]
fn test_verify_share_each_index_works() {
    // Same scheme, each of the three shares should independently verify.
    let secret: Field = 42;
    let coeff: Field = 7;
    let (xs, ys) = shares_for(secret, coeff);
    let address = get_secret_hash(secret);

    main(xs, ys, address, 3, 2, xs[0], ys[0]);
    main(xs, ys, address, 3, 2, xs[1], ys[1]);
    main(xs, ys, address, 3, 2, xs[2], ys[2]);
}

#[test(should_fail_with = "Provided share is not part of this SSS scheme")]
fn test_rejects_unrelated_provided_share() {
    let secret: Field = 0x1234;
    let coeff: Field = 0x5;
    let (xs, ys) = shares_for(secret, coeff);
    let address = get_secret_hash(secret);

    // (4, secret + coeff*4) is on the same line but at an x-coordinate
    // not held by any of the three real shares -- not a member share.
    main(xs, ys, address, 3, 2, 4, secret + coeff * 4);
}

#[test(should_fail_with = "Provided share is not part of this SSS scheme")]
fn test_rejects_provided_share_wrong_y() {
    // Provided x matches share 0, but y has been tampered with.
    let secret: Field = 0x1234;
    let coeff: Field = 0x5;
    let (xs, ys) = shares_for(secret, coeff);
    let address = get_secret_hash(secret);

    main(xs, ys, address, 3, 2, xs[0], ys[0] + 1);
}

#[test(should_fail_with = "Address is not the commitment of the secret")]
fn test_rejects_wrong_address() {
    let secret: Field = 99;
    let coeff: Field = 3;
    let (xs, ys) = shares_for(secret, coeff);
    // Address corresponds to a different secret.
    let wrong_address = get_secret_hash(secret + 1);

    main(xs, ys, wrong_address, 3, 2, xs[0], ys[0]);
}

#[test(should_fail_with = "Shares are not consistent with a degree-1 polynomial")]
fn test_rejects_inconsistent_shares() {
    // Shares 0 and 1 define the line `y = 0 + 10*x`; share 2 is off it.
    let xs: [Field; 3] = [1, 2, 3];
    let ys: [Field; 3] = [10, 20, 31]; // 31 should be 30
    // Address claims to commit to S = 0 (the line's constant term from
    // shares 0,1), but the inconsistency check must fire before then.
    let address = get_secret_hash(0);

    main(xs, ys, address, 3, 2, xs[0], ys[0]);
}

#[test(should_fail_with = "Shares 0 and 1 must have distinct x")]
fn test_rejects_duplicate_x_coords() {
    // Two shares share an x-coordinate; reconstruction is undefined.
    let xs: [Field; 3] = [1, 1, 3];
    let ys: [Field; 3] = [10, 10, 30];
    let address = get_secret_hash(0);

    main(xs, ys, address, 3, 2, xs[0], ys[0]);
}

#[test(should_fail_with = "verifyShare: only total = 3 is supported")]
fn test_rejects_wrong_total() {
    let secret: Field = 1;
    let coeff: Field = 1;
    let (xs, ys) = shares_for(secret, coeff);
    let address = get_secret_hash(secret);

    main(xs, ys, address, 4, 2, xs[0], ys[0]);
}

#[test(should_fail_with = "verifyShare: only threshold = 2 is supported")]
fn test_rejects_wrong_threshold() {
    let secret: Field = 1;
    let coeff: Field = 1;
    let (xs, ys) = shares_for(secret, coeff);
    let address = get_secret_hash(secret);

    main(xs, ys, address, 3, 3, xs[0], ys[0]);
}
