//! Pure, synchronous helpers re-exported from `aurum-domain` so the apps format
//! numbers exactly like the server does (no IO, no state).

use aurum_domain::decimal::Decimal;

use crate::error::AurumError;

/// Version of the Aurum API/contract this build was compiled against.
#[uniffi::export]
pub fn contract_version() -> String {
    aurum_domain::APP_VERSION.to_string()
}

/// Rounds a decimal string to `places` fractional digits (ROUND_HALF_EVEN, like
/// the server's `Decimal.quantize`) and prints it the way the API does:
/// `"1.005"`, 2 -> `"1.00"`. Accepts the API's own decimal strings.
#[uniffi::export]
pub fn decimal_quantize(value: String, places: u32) -> Result<String, AurumError> {
    let parsed: Decimal =
        value.parse().map_err(|_| AurumError::InvalidArgument { message: format!("not a decimal number: {value}") })?;
    Ok(parsed.quantize(-i64::from(places)).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantize_rounds_half_even_and_pads() {
        assert_eq!(decimal_quantize("1.005".into(), 2).unwrap(), "1.00");
        assert_eq!(decimal_quantize("1.015".into(), 2).unwrap(), "1.02");
        assert_eq!(decimal_quantize("5".into(), 2).unwrap(), "5.00");
        assert!(decimal_quantize("abc".into(), 2).is_err());
    }
}
