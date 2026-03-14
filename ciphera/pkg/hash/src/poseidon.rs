use acvm::{AcirField, BlackBoxResolutionError, FieldElement};
use bn254_blackbox_solver::poseidon2_permutation;

/// Poseidon2 sponge-based hash, equivalent to the one previously in bn254_blackbox_solver.
///
/// Removed upstream in Noir beta.19; re-implemented here using the still-exported
/// `poseidon2_permutation` primitive.
pub fn poseidon_hash(inputs: &[FieldElement]) -> Result<FieldElement, BlackBoxResolutionError> {
    let two_pow_64 = 18446744073709551616_u128.into();
    let iv = FieldElement::from(inputs.len()) * two_pow_64;
    let mut sponge = Poseidon2Sponge::new(iv, 3);
    for input in inputs.iter() {
        sponge.absorb(*input)?;
    }
    sponge.squeeze()
}

struct Poseidon2Sponge {
    rate: usize,
    squeezed: bool,
    cache: Vec<FieldElement>,
    state: Vec<FieldElement>,
}

impl Poseidon2Sponge {
    fn new(iv: FieldElement, rate: usize) -> Self {
        let mut result = Poseidon2Sponge {
            cache: Vec::with_capacity(rate),
            state: vec![FieldElement::zero(); rate + 1],
            squeezed: false,
            rate,
        };
        result.state[rate] = iv;
        result
    }

    fn perform_duplex(
        &mut self,
    ) -> Result<(), BlackBoxResolutionError> {
        // zero-pad the cache
        for _ in self.cache.len()..self.rate {
            self.cache.push(FieldElement::zero());
        }
        // add the cache into sponge state
        for i in 0..self.rate {
            self.state[i] += self.cache[i];
        }
        self.state = poseidon2_permutation(&self.state)?;
        Ok(())
    }

    fn absorb(
        &mut self,
        input: FieldElement,
    ) -> Result<(), BlackBoxResolutionError> {
        assert!(!self.squeezed);
        if self.cache.len() == self.rate {
            self.perform_duplex()?;
            self.cache = vec![input];
        } else {
            self.cache.push(input);
        }
        Ok(())
    }

    fn squeeze(
        &mut self,
    ) -> Result<FieldElement, BlackBoxResolutionError> {
        assert!(!self.squeezed);
        self.perform_duplex()?;
        self.squeezed = true;
        Ok(self.state[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_smoke_test() {
        let fields = [
            FieldElement::from(1u128),
            FieldElement::from(2u128),
            FieldElement::from(3u128),
            FieldElement::from(4u128),
        ];
        let result = poseidon_hash(&fields).expect("should hash successfully");
        // Known-good value from Noir's original test suite
        assert_eq!(
            result,
            FieldElement::from_hex(
                "130bf204a32cac1f0ace56c78b731aa3809f06df2731ebcf6b3464a15788b1b9"
            )
            .unwrap(),
        );
    }
}
