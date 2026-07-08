use element::Element;
use rand::RngCore;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use zk_primitives::{CitreaNetwork, Note, citrea_ctusd_note_kind, citrea_wcbtc_note_kind};

pub const WCBTC_TICKER: &str = "WCBTC";
pub const CITREA_USD_TICKER: &str = "CUSD";

/// Compile-time default Citrea network for the encode/decode helpers and
/// `From` conversions in this crate, which have no `--chain` argument in
/// scope (they sit behind fixed trait signatures). This is the single
/// source of truth: `note_url` and `main` re-import this constant rather
/// than redeclaring their own, so the three modules can never drift out of
/// sync again. It matches the CLI's default `--chain 5115` and the
/// testnet-only Citrea USD note kind (see `client::client_tests::CHAIN_ID`).
///
/// Call sites that *do* have a chain id available (e.g. the on-chain
/// slow-burn flow) should derive the network from it via
/// [`network_for_chain`] instead of reading this constant.
pub const CLI_NETWORK: CitreaNetwork = CitreaNetwork::Testnet;

/// Map the CLI's `--chain` argument to its Citrea network.
///
/// Unknown / devnet chain ids fall back to [`CLI_NETWORK`] (testnet),
/// matching the CLI's default (`--chain 5115`). Use this anywhere a chain id
/// is in scope so the selected network stays consistent with the `--chain`
/// flag instead of a hardcoded enum.
#[must_use]
pub fn network_for_chain(chain: u64) -> CitreaNetwork {
    CitreaNetwork::try_from_chain_id(chain).unwrap_or(CLI_NETWORK)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CitreaToken {
    WrappedCitreaBtc,
    CitreaUsd,
}

impl CitreaToken {
    #[must_use]
    pub const fn ticker(self) -> &'static str {
        match self {
            Self::WrappedCitreaBtc => WCBTC_TICKER,
            Self::CitreaUsd => CITREA_USD_TICKER,
        }
    }

    #[must_use]
    pub const fn currency_code(self) -> u8 {
        match self {
            Self::WrappedCitreaBtc => 1,
            Self::CitreaUsd => 2,
        }
    }

    #[must_use]
    pub fn note_kind(self, network: CitreaNetwork) -> Element {
        match self {
            Self::WrappedCitreaBtc => citrea_wcbtc_note_kind(network),
            // The rollup's pending Citrea USD token currently uses the
            // existing Citrea testnet USD fixture address. Keep this behind
            // the token registry so replacing the address is one local change
            // when the canonical token is registered on the rollup.
            Self::CitreaUsd => citrea_ctusd_note_kind(network),
        }
    }

    #[must_use]
    pub fn from_ticker(ticker: &str) -> Option<Self> {
        match ticker.trim().to_ascii_uppercase().as_str() {
            WCBTC_TICKER | "CBTC" | "WC-BTC" | "WRAPPEDCITREABTC" | "WRAPPED_CITREA_BTC" => {
                Some(Self::WrappedCitreaBtc)
            }
            CITREA_USD_TICKER | "CITREAUSD" | "CITREA_USD" | "CITREA-USD" | "USD" | "USDC" => {
                Some(Self::CitreaUsd)
            }
            _ => None,
        }
    }

    #[must_use]
    pub fn from_currency_code(currency: u8) -> Option<Self> {
        match currency {
            1 => Some(Self::WrappedCitreaBtc),
            2 => Some(Self::CitreaUsd),
            _ => None,
        }
    }
}

#[must_use]
pub fn normalize_citrea_ticker(ticker: &str) -> Option<&'static str> {
    CitreaToken::from_ticker(ticker).map(CitreaToken::ticker)
}

#[must_use]
pub fn supported_citrea_tokens(network: CitreaNetwork) -> Vec<(&'static str, Element)> {
    [CitreaToken::WrappedCitreaBtc, CitreaToken::CitreaUsd]
        .into_iter()
        .map(|token| (token.ticker(), token.note_kind(network)))
        .collect()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CipheraAddress {
    pub version: u8,
    pub currency: u8,
    pub public_key: Element,
    pub psi: Option<Element>,
    pub value: Element,
}

pub fn random_element() -> Element {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    Element::from_be_bytes(bytes)
}

/// Build the `(utxo_kind, note_kind)` pair for a token on a specific network.
///
/// `note_kind` is network-dependent for WCBTC (testnet and mainnet have
/// distinct bridge contracts), so callers that mint or otherwise *construct*
/// notes must pass the network derived from their `--chain` argument (see
/// [`network_for_chain`]) rather than relying on [`CLI_NETWORK`].
pub fn citrea_token_data(network: CitreaNetwork, ticker: &str) -> (Element, Element) {
    let token = CitreaToken::from_ticker(ticker)
        .unwrap_or_else(|| unreachable!("only WCBTC and CUSD tokens are supported"));
    (Element::new(2), token.note_kind(network))
}

/// True when `note_kind` is the WCBTC note kind on any supported network.
///
/// A `note_kind` fully encodes its network, so reverse lookups (note → ticker)
/// can resolve a token regardless of which `--chain` the wallet runs on by
/// checking every known network. The ticker itself is network-independent.
fn is_wcbtc_note_kind(note_kind: Element) -> bool {
    note_kind == citrea_wcbtc_note_kind(CitreaNetwork::Testnet)
        || note_kind == citrea_wcbtc_note_kind(CitreaNetwork::Mainnet)
}

fn is_cusd_note_kind(note_kind: Element) -> bool {
    note_kind == citrea_ctusd_note_kind(CitreaNetwork::Testnet)
        || note_kind == citrea_ctusd_note_kind(CitreaNetwork::Mainnet)
}

pub fn citrea_currency_from_contract(note_kind: Element) -> u8 {
    if is_wcbtc_note_kind(note_kind) {
        return CitreaToken::WrappedCitreaBtc.currency_code();
    }
    if is_cusd_note_kind(note_kind) {
        return CitreaToken::CitreaUsd.currency_code();
    }
    unreachable!("only WCBTC and CUSD tokens are supported")
}

pub fn citrea_ticker_from_contract(note_kind: Element) -> String {
    if is_wcbtc_note_kind(note_kind) {
        return WCBTC_TICKER.to_string();
    }
    if is_cusd_note_kind(note_kind) {
        return CITREA_USD_TICKER.to_string();
    }
    unreachable!("only WCBTC and CUSD tokens are supported")
}

impl From<&CipheraAddress> for Note {
    fn from(value: &CipheraAddress) -> Self {
        let psi = value.psi.unwrap_or_else(random_element);

        let contract = CitreaToken::from_currency_code(value.currency)
            .unwrap_or_else(|| unreachable!("currency code must be 1 or 2"))
            .note_kind(CLI_NETWORK);

        Note {
            utxo_kind: Element::new(2),
            note_kind: contract,
            address: value.public_key,
            psi,
            value: value.value,
        }
    }
}

impl From<&Note> for CipheraAddress {
    fn from(note: &Note) -> Self {
        Self {
            version: 0,
            currency: citrea_currency_from_contract(note.note_kind),
            public_key: note.address,
            psi: Some(note.psi),
            value: note.value,
        }
    }
}

impl CipheraAddress {
    #[must_use]
    pub fn address(&self) -> Element {
        Note::from(self).address
    }

    #[must_use]
    pub fn commitment(&self) -> Element {
        Note::from(self).commitment()
    }

    #[must_use]
    pub fn psi(&self) -> Element {
        match self.version {
            0 => self.psi.expect("version 1 should have explicit psi"),
            _ => unreachable!("only version 1, 2 or 3 is supported"),
        }
    }

    #[must_use]
    pub fn encode_address(&self) -> String {
        let mut bytes = Vec::new();

        bytes.push(self.version);
        bytes.push(self.currency);

        bytes.extend_from_slice(&self.public_key.to_be_bytes());

        if let Some(psi) = &self.psi {
            if self.version == 0 {
                bytes.extend_from_slice(&psi.to_be_bytes());
            }
        }

        let value_bytes = self.value.to_be_bytes();
        let leading_zeros = value_bytes.iter().take_while(|&&b| b == 0).count();
        #[allow(clippy::cast_possible_truncation)]
        bytes.push(leading_zeros as u8);
        bytes.extend_from_slice(&value_bytes[leading_zeros..]);

        bs58::encode(bytes).into_string()
    }
}

#[must_use]
pub fn decode_address(address: &str) -> CipheraAddress {
    match try_decode_address(address) {
        Ok(a) => a,
        Err(e) => panic!("Failed to decode Ciphera address: {e}"),
    }
}

pub fn try_decode_address(address: &str) -> Result<CipheraAddress, String> {
    let address_bytes = bs58::decode(address)
        .into_vec()
        .map_err(|e| format!("failed to decode base58 payload: {e}"))?;

    let mut rest = &address_bytes[..];

    if rest.len() < 2 {
        return Err("address payload is too short".to_string());
    }
    let version = rest[0];
    let currency = rest[1];
    rest = &rest[2..];

    // Reject unknown currency bytes here so callers get a clean error instead
    // of hitting `unreachable!("currency code must be 1 or 2")` inside
    // `Note::from(&CipheraAddress)` when `.address()`/`.commitment()` is called.
    if CitreaToken::from_currency_code(currency).is_none() {
        return Err(format!(
            "unsupported currency code {currency}; expected 1 (WCBTC) or 2 (CUSD)"
        ));
    }

    if rest.len() < 32 {
        return Err("not enough bytes for public_key".to_string());
    }
    let public_key_bytes: [u8; 32] = rest[..32]
        .try_into()
        .map_err(|_| "not enough bytes for public_key".to_string())?;

    let public_key = Element::from_be_bytes(public_key_bytes);
    rest = &rest[32..];

    let psi = match version {
        0 => {
            if rest.len() < 32 {
                return Err("not enough bytes for psi".to_string());
            }
            let psi_bytes: [u8; 32] = rest[..32]
                .try_into()
                .map_err(|_| "not enough bytes for psi".to_string())?;
            rest = &rest[32..];
            Some(Element::from_be_bytes(psi_bytes))
        }
        _ => return Err(format!("unsupported address version {version}")),
    };

    if rest.is_empty() {
        return Err("missing value length prefix".to_string());
    }
    let leading_zeros = rest[0] as usize;
    if leading_zeros > 32 {
        return Err(format!(
            "invalid leading zero count {leading_zeros}; expected <= 32"
        ));
    }
    rest = &rest[1..];

    let value_len = 32 - leading_zeros;
    if rest.len() != value_len {
        return Err(format!(
            "invalid value length: expected {value_len}, got {}",
            rest.len()
        ));
    }
    let value_without_leading_zeros = &rest[..value_len];
    //rest = &rest[value_len..];

    let mut value_bytes = [0u8; 32];
    value_bytes[leading_zeros..].copy_from_slice(value_without_leading_zeros);
    let value = Element::from_be_bytes(value_bytes);

    Ok(CipheraAddress {
        version,
        currency,
        public_key,
        psi,
        value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hash::hash_merge;
    use zk_primitives::Note;

    #[test]
    fn test_roundtrip_from_wcbtc_note() {
        let note = Note {
            utxo_kind: Element::new(2),
            note_kind: citrea_wcbtc_note_kind(CLI_NETWORK),
            address: hash_merge([Element::new(101), Element::ZERO]),
            psi: Element::ZERO,
            value: Element::new(1),
        };

        let a: CipheraAddress = (&note).into();

        println!("to be encoded: {a:?}");

        let encoded = a.encode_address();

        println!("encoded: {encoded}");

        let decoded_note = Note::from(&decode_address(&encoded));

        println!("decoded: {decoded_note:?}");

        // Verify
        assert_eq!(decoded_note.utxo_kind, note.utxo_kind);
        assert_eq!(decoded_note.note_kind, note.note_kind);
        assert_eq!(decoded_note.value, note.value);
        assert_eq!(decoded_note.address, note.address);
        assert_eq!(decoded_note.psi, note.psi);
    }

    #[test]
    fn test_roundtrip_from_cusd_note() {
        let note = Note {
            utxo_kind: Element::new(2),
            note_kind: citrea_ctusd_note_kind(CitreaNetwork::Testnet),
            address: hash_merge([Element::new(101), Element::ZERO]),
            psi: Element::ZERO,
            value: Element::new(1),
        };

        let a: CipheraAddress = (&note).into();

        println!("to be encoded: {a:?}");

        let encoded = a.encode_address();

        println!("encoded: {encoded}");

        let decoded_note = Note::from(&decode_address(&encoded));

        println!("decoded: {decoded_note:?}");

        // Verify
        assert_eq!(decoded_note.utxo_kind, note.utxo_kind);
        assert_eq!(decoded_note.note_kind, note.note_kind);
        assert_eq!(decoded_note.value, note.value);
        assert_eq!(decoded_note.address, note.address);
        assert_eq!(decoded_note.psi, note.psi);
    }

    #[test]
    fn test_try_decode_address_rejects_unknown_currency() {
        // A structurally valid address whose currency byte is neither 1 (WCBTC)
        // nor 2 (CUSD) must surface a clean Err, not panic in Note::from.
        let addr = CipheraAddress {
            version: 0,
            currency: 3,
            public_key: hash_merge([Element::new(101), Element::ZERO]),
            psi: Some(Element::ZERO),
            value: Element::new(1),
        };
        let encoded = addr.encode_address();

        let err = try_decode_address(&encoded)
            .expect_err("unknown currency byte must be rejected, not decoded");
        assert!(
            err.contains("currency"),
            "error should mention the currency code; got: {err}"
        );
    }

    #[test]
    fn test_citrea_usd_aliases_normalize_to_cusd() {
        for ticker in ["CUSD", "citreausd", "Citrea_USD", "USDC", "usd"] {
            assert_eq!(normalize_citrea_ticker(ticker), Some(CITREA_USD_TICKER));
        }
    }
}
