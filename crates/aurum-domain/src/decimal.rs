//! `Decimal` — the money/quantity type of the API contract.
//!
//! The Python backend uses `decimal.Decimal` end to end (asyncpg hands back
//! `Numeric` columns as `Decimal`, services do arithmetic on them, pydantic
//! serialises them with `str()`). Matching its JSON output byte for byte
//! therefore needs three things this type provides and a plain
//! `rust_decimal`/`BigDecimal` does not:
//!
//! * **Representation is preserved.** `Decimal("1000.00")` and
//!   `Decimal("1000")` are equal but print differently, and the exponent
//!   (number of trailing zeros) is part of the output. The value is stored as
//!   an arbitrary-precision coefficient + exponent (a `BigDecimal`), so
//!   `numeric(38,18)` crypto quantities never overflow either.
//! * **`Display` is Python's `Decimal.__str__`**, including its switch to
//!   scientific notation (`0E-18`, `1E+2`, `1.0E-7`).
//! * **Arithmetic follows Python's default decimal context** (28 significant
//!   digits, ROUND_HALF_EVEN, "ideal exponent" rules) via [`Decimal::add`],
//!   [`Decimal::mul`], [`Decimal::div`], [`Decimal::quantize`] — so a value
//!   computed in a service prints exactly what the Python service printed.
//!
//! Serde: serialises as a JSON *string* (pydantic v2's JSON mode does the
//! same); deserialises from a string or a number (clients only — request
//! bodies go through `validation`, which reproduces pydantic's errors).

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use bigdecimal::BigDecimal;
use num_bigint::{BigInt, Sign};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Precision of Python's default decimal context (`getcontext().prec`).
pub const PY_CONTEXT_PRECISION: u64 = 28;

#[derive(Clone, Default)]
pub struct Decimal(BigDecimal);

impl Decimal {
    /// Zero with exponent 0 — what Python's `Decimal("0")` / `Decimal(0)` is,
    /// and what `sum()` of an empty list of Decimals serialises as ("0").
    pub fn zero() -> Self {
        Self(BigDecimal::from(0))
    }

    /// Builds `coefficient × 10^exponent` without normalising anything.
    pub fn from_parts(coefficient: BigInt, exponent: i64) -> Self {
        Self(BigDecimal::new(coefficient, -exponent))
    }

    /// (signed coefficient, exponent) — Python's `as_tuple()` minus the sign split.
    pub fn parts(&self) -> (BigInt, i64) {
        let (coefficient, scale) = self.0.as_bigint_and_exponent();
        (coefficient, -scale)
    }

    pub fn exponent(&self) -> i64 {
        self.parts().1
    }

    pub fn as_bigdecimal(&self) -> &BigDecimal {
        &self.0
    }

    pub fn into_bigdecimal(self) -> BigDecimal {
        self.0
    }

    pub fn is_zero(&self) -> bool {
        self.parts().0.sign() == Sign::NoSign
    }

    pub fn is_negative(&self) -> bool {
        self.parts().0.sign() == Sign::Minus
    }

    /// Python's `float(d)`: correctly rounded from the decimal string, which
    /// is what CPython does. (Going through `to_f64()` of a binary type can
    /// be off by one ulp, which then shows up in a `float` response field.)
    pub fn to_f64(&self) -> f64 {
        self.to_string().parse().unwrap_or(f64::NAN)
    }

    /// Python's `abs(d)` (keeps the exponent).
    pub fn abs(&self) -> Self {
        let (c, e) = self.parts();
        Self::from_parts(if c.sign() == Sign::Minus { -c } else { c }, e)
    }

    /// Python's unary minus under the default context (exact here).
    pub fn neg(&self) -> Self {
        let (c, e) = self.parts();
        Self::from_parts(-c, e)
    }

    /// `self + other` with Python's default context.
    pub fn add(&self, other: &Self) -> Self {
        let (a, ea) = self.parts();
        let (b, eb) = other.parts();
        let exp = ea.min(eb);
        let sum = a * pow10((ea - exp) as u64) + b * pow10((eb - exp) as u64);
        Self::fix(sum, exp)
    }

    /// `self - other` with Python's default context.
    pub fn sub(&self, other: &Self) -> Self {
        self.add(&other.neg())
    }

    /// `self * other` with Python's default context.
    pub fn mul(&self, other: &Self) -> Self {
        let (a, ea) = self.parts();
        let (b, eb) = other.parts();
        Self::fix(a * b, ea + eb)
    }

    /// `self / other` with Python's default context — a port of CPython's
    /// `_pydecimal.Decimal.__truediv__` (exact results keep the ideal
    /// exponent, inexact ones are rounded to 28 significant digits).
    ///
    /// Returns `None` on division by zero (Python raises
    /// `DivisionByZero`/`InvalidOperation`; callers mirror whatever the
    /// Python code did around it, usually an `if divisor else ...` guard).
    pub fn div(&self, other: &Self) -> Option<Self> {
        let (b, eb) = other.parts();
        if b.sign() == Sign::NoSign {
            return None;
        }
        let (a, ea) = self.parts();
        let negative = (a.sign() == Sign::Minus) != (b.sign() == Sign::Minus);
        let a_abs = a.magnitude().clone();
        let b_abs = b.magnitude().clone();
        if a_abs == num_bigint::BigUint::from(0u8) {
            // Python keeps the sign of a zero result; we can't represent -0,
            // which only matters for "-0" output (documented in PORTING.md).
            return Some(Self::from_parts(BigInt::from(0), ea - eb));
        }
        let prec = PY_CONTEXT_PRECISION as i64;
        let shift = digit_count(&b_abs) as i64 - digit_count(&a_abs) as i64 + prec + 1;
        let mut exp = ea - eb - shift;
        let (mut coeff, remainder) = if shift >= 0 {
            let n = a_abs * pow10u(shift as u64);
            (&n / &b_abs, &n % &b_abs)
        } else {
            let d = &b_abs * pow10u((-shift) as u64);
            (&a_abs / &d, &a_abs % &d)
        };
        let zero = num_bigint::BigUint::from(0u8);
        if remainder != zero {
            if &coeff % 5u8 == zero {
                coeff += 1u8;
            }
        } else {
            let ideal = ea - eb;
            let ten = num_bigint::BigUint::from(10u8);
            while exp < ideal && &coeff % &ten == zero {
                coeff /= &ten;
                exp += 1;
            }
        }
        let signed = BigInt::from_biguint(if negative { Sign::Minus } else { Sign::Plus }, coeff);
        Some(Self::fix(signed, exp))
    }

    /// Python's `d.quantize(Decimal(10) ** exponent)` with ROUND_HALF_EVEN
    /// (the context default) — e.g. `quantize(-2)` is `.quantize(Decimal("0.01"))`.
    pub fn quantize(&self, exponent: i64) -> Self {
        let (c, e) = self.parts();
        if e >= exponent {
            return Self::from_parts(c * pow10((e - exponent) as u64), exponent);
        }
        let drop = (exponent - e) as u64;
        Self::from_parts(round_half_even(&c, drop), exponent)
    }

    /// Python's `round(d)` → int (ROUND_HALF_EVEN).
    pub fn round_to_int(&self) -> BigInt {
        self.quantize(0).parts().0
    }

    /// Python's `Decimal.normalize()` (strip trailing zeros; zero → "0").
    pub fn normalize(&self) -> Self {
        let (mut c, mut e) = self.parts();
        if c.sign() == Sign::NoSign {
            return Self::zero();
        }
        let ten = BigInt::from(10);
        let zero = BigInt::from(0);
        while &c % &ten == zero {
            c /= &ten;
            e += 1;
        }
        Self::from_parts(c, e)
    }

    /// Python's `_fix()` for the default context: round to 28 significant
    /// digits with ROUND_HALF_EVEN.
    fn fix(coefficient: BigInt, exponent: i64) -> Self {
        let digits = digit_count(coefficient.magnitude()) as u64;
        if digits <= PY_CONTEXT_PRECISION {
            return Self::from_parts(coefficient, exponent);
        }
        let drop = digits - PY_CONTEXT_PRECISION;
        let rounded = round_half_even(&coefficient, drop);
        // Rounding 9999… up can add a digit; drop one more in that case.
        if digit_count(rounded.magnitude()) as u64 > PY_CONTEXT_PRECISION {
            return Self::from_parts(round_half_even(&rounded, 1), exponent + drop as i64 + 1);
        }
        Self::from_parts(rounded, exponent + drop as i64)
    }
}

fn pow10(n: u64) -> BigInt {
    BigInt::from(10).pow(n as u32)
}

fn pow10u(n: u64) -> num_bigint::BigUint {
    num_bigint::BigUint::from(10u8).pow(n as u32)
}

fn digit_count(n: &num_bigint::BigUint) -> usize {
    let s = n.to_str_radix(10);
    if s == "0" { 1 } else { s.len() }
}

/// Divides `c` by 10^drop, rounding half to even.
fn round_half_even(c: &BigInt, drop: u64) -> BigInt {
    if drop == 0 {
        return c.clone();
    }
    let divisor = pow10(drop);
    let negative = c.sign() == Sign::Minus;
    let magnitude = BigInt::from_biguint(Sign::Plus, c.magnitude().clone());
    let quotient = &magnitude / &divisor;
    let remainder = &magnitude % &divisor;
    let twice: BigInt = &remainder * BigInt::from(2);
    let round_up = match twice.cmp(&divisor) {
        Ordering::Greater => true,
        Ordering::Less => false,
        Ordering::Equal => &quotient % BigInt::from(2) == BigInt::from(1),
    };
    let quotient = if round_up { quotient + BigInt::from(1) } else { quotient };
    if negative { -quotient } else { quotient }
}

/// Python's `Decimal.__str__` (eng=False, capitals=1).
impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (c, exp) = self.parts();
        let sign = if c.sign() == Sign::Minus { "-" } else { "" };
        let int = c.magnitude().to_str_radix(10);
        let len = int.len() as i64;
        let leftdigits = exp + len;
        let dotplace = if exp <= 0 && leftdigits > -6 { leftdigits } else { 1 };
        let (intpart, fracpart) = if dotplace <= 0 {
            ("0".to_string(), format!(".{}{}", "0".repeat((-dotplace) as usize), int))
        } else if dotplace >= len {
            (format!("{}{}", int, "0".repeat((dotplace - len) as usize)), String::new())
        } else {
            let (head, tail) = int.split_at(dotplace as usize);
            (head.to_string(), format!(".{tail}"))
        };
        let exp_part = if leftdigits == dotplace { String::new() } else { format!("E{:+}", leftdigits - dotplace) };
        write!(f, "{sign}{intpart}{fracpart}{exp_part}")
    }
}

impl fmt::Debug for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Decimal('{self}')")
    }
}

/// Parses the same literals Python's `Decimal(str)` accepts for finite
/// values (leading/trailing whitespace, `_` separators, exponent), keeping
/// the representation (`"1.10"` stays scale 2, `"1e3"` stays `1E+3`).
impl FromStr for Decimal {
    type Err = ParseDecimalError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let s = s.replace('_', "");
        let (mantissa, exp) = match s.find(['e', 'E']) {
            Some(i) => {
                let e: i64 = s[i + 1..].parse().map_err(|_| ParseDecimalError)?;
                (&s[..i], e)
            }
            None => (s.as_str(), 0),
        };
        let (negative, mantissa) = match mantissa.as_bytes().first() {
            Some(b'-') => (true, &mantissa[1..]),
            Some(b'+') => (false, &mantissa[1..]),
            _ => (false, mantissa),
        };
        let (int_part, frac_part) = match mantissa.find('.') {
            Some(i) => (&mantissa[..i], &mantissa[i + 1..]),
            None => (mantissa, ""),
        };
        if int_part.is_empty() && frac_part.is_empty() {
            return Err(ParseDecimalError);
        }
        if !int_part.bytes().chain(frac_part.bytes()).all(|b| b.is_ascii_digit()) {
            return Err(ParseDecimalError);
        }
        let digits = format!("{int_part}{frac_part}");
        let coefficient =
            BigInt::from_str(if digits.is_empty() { "0" } else { &digits }).map_err(|_| ParseDecimalError)?;
        let coefficient = if negative { -coefficient } else { coefficient };
        Ok(Self::from_parts(coefficient, exp - frac_part.len() as i64))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid decimal literal")]
pub struct ParseDecimalError;

impl From<i64> for Decimal {
    fn from(value: i64) -> Self {
        Self(BigDecimal::from(value))
    }
}

impl From<i32> for Decimal {
    fn from(value: i32) -> Self {
        Self(BigDecimal::from(value))
    }
}

impl From<BigDecimal> for Decimal {
    fn from(value: BigDecimal) -> Self {
        Self(value)
    }
}

impl From<Decimal> for BigDecimal {
    fn from(value: Decimal) -> Self {
        value.0
    }
}

/// Numeric equality, like Python (`Decimal("1.0") == Decimal("1.00")`).
impl PartialEq for Decimal {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for Decimal {}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Decimal {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl Serialize for Decimal {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Decimal {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Str(String),
            Int(i64),
            Float(f64),
        }
        let text = match Raw::deserialize(deserializer)? {
            Raw::Str(s) => s,
            Raw::Int(i) => i.to_string(),
            // pydantic-core turns floats into Decimals through Rust's own
            // `f64` Display ("100" for 100.0) — reuse it for symmetry.
            Raw::Float(f) => f.to_string(),
        };
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(feature = "sqlx")]
mod sqlx_impl {
    use super::Decimal;
    use bigdecimal::BigDecimal;
    use sqlx::encode::IsNull;
    use sqlx::error::BoxDynError;
    use sqlx::postgres::{PgArgumentBuffer, PgHasArrayType, PgTypeInfo, PgValueRef, Postgres};

    impl sqlx::Type<Postgres> for Decimal {
        fn type_info() -> PgTypeInfo {
            <BigDecimal as sqlx::Type<Postgres>>::type_info()
        }

        fn compatible(ty: &PgTypeInfo) -> bool {
            <BigDecimal as sqlx::Type<Postgres>>::compatible(ty)
        }
    }

    impl PgHasArrayType for Decimal {
        fn array_type_info() -> PgTypeInfo {
            <BigDecimal as PgHasArrayType>::array_type_info()
        }
    }

    // Hand-written codec instead of delegating to BigDecimal's: sqlx's
    // BigDecimal conversion drops/changes the display scale (`1000.00` came
    // back as `1000`, `100.50` as `100.5000`), and that scale is exactly what
    // Python prints. See `pg_numeric` below.
    impl<'r> sqlx::Decode<'r, Postgres> for Decimal {
        fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
            match value.format() {
                sqlx::postgres::PgValueFormat::Binary => Ok(super::pg_numeric::decode(value.as_bytes()?)?),
                sqlx::postgres::PgValueFormat::Text => Ok(value.as_str()?.parse::<Decimal>()?),
            }
        }
    }

    impl sqlx::Encode<'_, Postgres> for Decimal {
        fn encode_by_ref(&self, buf: &mut PgArgumentBuffer) -> Result<IsNull, BoxDynError> {
            buf.extend_from_slice(&super::pg_numeric::encode(self));
            Ok(IsNull::No)
        }
    }
}

/// Postgres `numeric` binary wire format, preserving the display scale
/// (`dscale`) both ways — which is what asyncpg does and what makes
/// `numeric(14,2)` values print as `"1000.00"` rather than `"1000"`.
///
/// Layout: `ndigits: i16, weight: i16, sign: u16, dscale: u16`, then
/// `ndigits` base-10000 digits (`i16`); value = Σ dᵢ·10000^(weight−i).
///
/// Only the `sqlx` impls above call into it, so builds without that feature
/// (the CLI, the FFI crate) would otherwise warn; the unit tests still use it.
#[cfg_attr(not(feature = "sqlx"), allow(dead_code))]
pub(crate) mod pg_numeric {
    use super::{Decimal, ParseDecimalError, pow10};
    use num_bigint::{BigInt, Sign};

    const SIGN_POS: u16 = 0x0000;
    const SIGN_NEG: u16 = 0x4000;

    pub fn decode(bytes: &[u8]) -> Result<Decimal, ParseDecimalError> {
        let word = |i: usize| -> Result<u16, ParseDecimalError> {
            bytes.get(i * 2..i * 2 + 2).map(|b| u16::from_be_bytes([b[0], b[1]])).ok_or(ParseDecimalError)
        };
        let ndigits = word(0)? as usize;
        let weight = word(1)? as i16 as i64;
        let sign = word(2)?;
        let dscale = word(3)? as i64;
        if sign != SIGN_POS && sign != SIGN_NEG {
            // NaN / ±Infinity: not representable (and never stored by Aurum).
            return Err(ParseDecimalError);
        }
        let mut coefficient = BigInt::from(0);
        for i in 0..ndigits {
            coefficient = coefficient * 10_000 + BigInt::from(word(4 + i)?);
        }
        // `coefficient` is the value scaled by 10000^(ndigits-1-weight).
        let exponent = if ndigits == 0 { 0 } else { -4 * (ndigits as i64 - 1 - weight) };
        let target = -dscale;
        let coefficient = if exponent < target {
            coefficient / pow10((target - exponent) as u64)
        } else {
            coefficient * pow10((exponent - target) as u64)
        };
        let coefficient = if sign == SIGN_NEG { -coefficient } else { coefficient };
        Ok(Decimal::from_parts(coefficient, target))
    }

    pub fn encode(value: &Decimal) -> Vec<u8> {
        let (coefficient, exponent) = value.parts();
        let (coefficient, dscale) =
            if exponent > 0 { (coefficient * pow10(exponent as u64), 0u64) } else { (coefficient, (-exponent) as u64) };
        let negative = coefficient.sign() == Sign::Minus;
        let digits = coefficient.magnitude().to_str_radix(10);
        // Split into integer / fraction parts at `dscale`.
        let digits = if (digits.len() as u64) <= dscale {
            format!("{}{}", "0".repeat((dscale as usize + 1) - digits.len()), digits)
        } else {
            digits
        };
        let (int_part, frac_part) = digits.split_at(digits.len() - dscale as usize);
        let int_pad = (4 - int_part.len() % 4) % 4;
        let frac_pad = (4 - frac_part.len() % 4) % 4;
        let int_part = format!("{}{}", "0".repeat(int_pad), int_part);
        let frac_part = format!("{}{}", frac_part, "0".repeat(frac_pad));
        let mut groups: Vec<i16> = int_part
            .as_bytes()
            .chunks(4)
            .chain(frac_part.as_bytes().chunks(4))
            .map(|c| std::str::from_utf8(c).unwrap_or("0").parse().unwrap_or(0))
            .collect();
        let mut weight = (int_part.len() / 4) as i64 - 1;
        while groups.first() == Some(&0) {
            groups.remove(0);
            weight -= 1;
        }
        while groups.last() == Some(&0) {
            groups.pop();
        }
        if groups.is_empty() {
            weight = 0;
        }
        let mut out = Vec::with_capacity(8 + groups.len() * 2);
        out.extend_from_slice(&(groups.len() as i16).to_be_bytes());
        out.extend_from_slice(&(weight as i16).to_be_bytes());
        out.extend_from_slice(&(if negative && !groups.is_empty() { SIGN_NEG } else { SIGN_POS }).to_be_bytes());
        out.extend_from_slice(&(dscale as u16).to_be_bytes());
        for g in groups {
            out.extend_from_slice(&g.to_be_bytes());
        }
        out
    }
}

#[cfg(feature = "openapi")]
mod openapi_impl {
    use super::Decimal;
    use utoipa::openapi::schema::{ObjectBuilder, Schema, Type};
    use utoipa::openapi::{RefOr, schema::SchemaFormat};

    impl utoipa::PartialSchema for Decimal {
        fn schema() -> RefOr<Schema> {
            ObjectBuilder::new()
                .schema_type(Type::String)
                .format(Some(SchemaFormat::Custom("decimal".into())))
                .description(Some("Decimal serialised as a string, exactly like Python's str(Decimal)"))
                .into()
        }
    }

    impl utoipa::ToSchema for Decimal {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    #[test]
    fn display_matches_python_str() {
        for (input, expected) in [
            ("1000.00", "1000.00"),
            ("0", "0"),
            ("0.000000000000000000", "0E-18"),
            ("0.000000100000000000", "1.00000000000E-7"),
            ("1e3", "1E+3"),
            ("1E+2", "1E+2"),
            ("123.4560", "123.4560"),
            ("-12.5", "-12.5"),
            ("0.000001", "0.000001"),
            ("0.0000001", "1E-7"),
            ("-0.00", "0.00"),
            ("12345678901234567890.123456789012345678", "12345678901234567890.123456789012345678"),
        ] {
            assert_eq!(d(input).to_string(), expected, "{input}");
        }
    }

    #[test]
    fn division_matches_python_context() {
        assert_eq!(d("1").div(&d("3")).unwrap().to_string(), "0.3333333333333333333333333333");
        assert_eq!(d("10").div(&d("3")).unwrap().to_string(), "3.333333333333333333333333333");
        assert_eq!(d("1.00").div(&d("4")).unwrap().to_string(), "0.25");
        assert_eq!(d("10").div(&d("2")).unwrap().to_string(), "5");
        assert_eq!(d("1.00").div(&d("1")).unwrap().to_string(), "1.00");
        assert_eq!(d("100").div(&d("0.5")).unwrap().to_string(), "2.0E+2");
        assert_eq!(d("2").div(&d("3")).unwrap().to_string(), "0.6666666666666666666666666667");
        assert!(d("1").div(&d("0.00")).is_none());
    }

    #[test]
    fn add_mul_quantize() {
        assert_eq!(Decimal::zero().add(&d("1.50")).to_string(), "1.50");
        assert_eq!(d("1.10").mul(&d("3")).to_string(), "3.30");
        assert_eq!(d("2.675").quantize(-2).to_string(), "2.68");
        assert_eq!(d("2.665").quantize(-2).to_string(), "2.66");
        assert_eq!(d("-2.665").quantize(-2).to_string(), "-2.66");
        assert_eq!(d("5").quantize(-2).to_string(), "5.00");
        assert_eq!(d("1.2300").normalize().to_string(), "1.23");
        assert_eq!(
            d("1234567890.123456789").mul(&d("1234567890.123456789")).to_string(),
            "1524157875323883675.019051999"
        );
    }

    #[test]
    fn pg_numeric_roundtrip_keeps_scale() {
        for text in [
            "1000.00",
            "100.50",
            "0E-18",
            "0",
            "-12.345",
            "1E+2",
            "12345678901234567890.123456789012345678",
            "0.0001",
            "-0.5",
        ] {
            let value = d(text);
            let back = pg_numeric::decode(&pg_numeric::encode(&value)).unwrap();
            let expected = if text == "1E+2" { "100".to_string() } else { value.to_string() };
            assert_eq!(back.to_string(), expected, "{text}");
        }
        // 1000.00 as Postgres sends it: 1 digit (1000), weight 0, dscale 2.
        let wire = [0u8, 1, 0, 0, 0, 0, 0, 2, 0x03, 0xE8];
        assert_eq!(pg_numeric::decode(&wire).unwrap().to_string(), "1000.00");
    }

    #[test]
    fn float_conversion() {
        assert_eq!(d("33.33333333333333333333333333").to_f64(), 33.333_333_333_333_336);
        assert_eq!(d("1E+2").to_f64(), 100.0);
    }
}
