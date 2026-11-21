//! Request validation that reproduces FastAPI/pydantic v2 422 responses.
//!
//! FastAPI answers an invalid request with
//! `{"detail": [{"type", "loc", "msg", "input", "ctx"?}, ...]}`: one entry
//! per failing field, in field-definition order, `loc` prefixed with the
//! source (`"path"`, `"query"`, `"body"`). Clients (the web UI, the CLI and
//! the black-box test suite) read these, so request DTOs are not parsed with
//! `serde::Deserialize` (whose errors look nothing like this) but through
//! [`FromInput`], written with the small combinator API below:
//!
//! ```
//! use aurum_domain::validation::{FromInput, Val, ValidationErrors};
//!
//! pub struct TagCreate { pub name: String }
//!
//! impl FromInput for TagCreate {
//!     fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
//!         let mut o = v.object()?;
//!         let name = o.required("name", |v| v.str()?.min_len(1)?.max_len(50));
//!         o.finish()?;
//!         Ok(Self { name: name.value() })
//!     }
//! }
//! ```
//!
//! Coercions follow pydantic's *lax* mode (what FastAPI uses): `"7"` is a
//! valid int, `true` is a valid int, `5.0` is a valid int but `5.5` is
//! `int_from_float`, a float becomes a Decimal through Rust's `f64` Display
//! (exactly what pydantic-core does), and so on. Each error constructor in
//! [`err`] carries pydantic's exact `type`/`msg`/`ctx` text.

use std::collections::HashMap;
use std::fmt;
use std::sync::{LazyLock, Mutex};

use chrono::{NaiveDate, NaiveDateTime};
use regex::Regex;
use serde::{Serialize, Serializer};
use serde_json::{Map, Value};

use crate::decimal::Decimal;
use crate::pyfmt::py_float_repr;

// ---------------------------------------------------------------------------
// Error model
// ---------------------------------------------------------------------------

/// One `loc` element: a field name or a list index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum LocItem {
    Key(String),
    Index(usize),
}

impl From<&str> for LocItem {
    fn from(value: &str) -> Self {
        Self::Key(value.to_string())
    }
}

impl From<usize> for LocItem {
    fn from(value: usize) -> Self {
        Self::Index(value)
    }
}

pub type Loc = Vec<LocItem>;

/// A JSON value echoed back in an error (`input`/`ctx`), printed with
/// Python's float formatting (`1000.0`, `1e-05`) like FastAPI does.
#[derive(Debug, Clone, PartialEq)]
pub struct PyJson(pub Value);

impl Serialize for PyJson {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};
        match &self.0 {
            Value::Number(n) if n.is_f64() => {
                let raw = serde_json::value::RawValue::from_string(py_float_repr(n.as_f64().unwrap_or_default()))
                    .map_err(serde::ser::Error::custom)?;
                raw.serialize(serializer)
            }
            Value::Array(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(&PyJson(item.clone()))?;
                }
                seq.end()
            }
            Value::Object(map) => {
                let mut out = serializer.serialize_map(Some(map.len()))?;
                for (k, v) in map {
                    out.serialize_entry(k, &PyJson(v.clone()))?;
                }
                out.end()
            }
            other => other.serialize(serializer),
        }
    }
}

/// One entry of FastAPI's `detail` list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ErrorItem {
    #[serde(rename = "type")]
    pub kind: String,
    pub loc: Loc,
    pub msg: String,
    pub input: PyJson,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ctx: Option<PyJson>,
}

impl ErrorItem {
    pub fn new(kind: &str, loc: Loc, msg: impl Into<String>, input: Value) -> Self {
        Self { kind: kind.to_string(), loc, msg: msg.into(), input: PyJson(input), ctx: None }
    }

    pub fn with_ctx(mut self, ctx: Value) -> Self {
        self.ctx = Some(PyJson(ctx));
        self
    }
}

/// The full `detail` list of a 422.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ValidationErrors(pub Vec<ErrorItem>);

impl ValidationErrors {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn extend(&mut self, other: ValidationErrors) {
        self.0.extend(other.0);
    }
}

impl From<ErrorItem> for ValidationErrors {
    fn from(value: ErrorItem) -> Self {
        Self(vec![value])
    }
}

impl fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} validation error(s)", self.0.len())
    }
}

impl std::error::Error for ValidationErrors {}

/// pydantic error constructors — `type`, `msg` and `ctx` exactly as
/// pydantic-core 2.x words them.
pub mod err {
    use super::*;
    use serde_json::json;

    /// FastAPI's `jsonable_encoder(Decimal)`: int when the exponent is
    /// non-negative, float otherwise (used for `ctx` of Decimal constraints).
    pub fn decimal_ctx(d: &Decimal) -> Value {
        if d.exponent() >= 0 {
            d.to_string().parse::<i64>().map(Value::from).unwrap_or_else(|_| json!(d.to_f64()))
        } else {
            json!(d.to_f64())
        }
    }

    pub fn missing(loc: Loc, input: Value) -> ErrorItem {
        ErrorItem::new("missing", loc, "Field required", input)
    }
    pub fn model_attributes_type(loc: Loc, input: Value) -> ErrorItem {
        ErrorItem::new(
            "model_attributes_type",
            loc,
            "Input should be a valid dictionary or object to extract fields from",
            input,
        )
    }
    pub fn json_invalid(position: usize, error: &str) -> ErrorItem {
        ErrorItem::new("json_invalid", vec!["body".into(), position.into()], "JSON decode error", json!({}))
            .with_ctx(json!({ "error": error }))
    }
    pub fn string_type(loc: Loc, input: Value) -> ErrorItem {
        ErrorItem::new("string_type", loc, "Input should be a valid string", input)
    }
    pub fn string_too_short(loc: Loc, input: Value, min: usize) -> ErrorItem {
        let unit = if min == 1 { "character" } else { "characters" };
        ErrorItem::new("string_too_short", loc, format!("String should have at least {min} {unit}"), input)
            .with_ctx(json!({ "min_length": min }))
    }
    pub fn string_too_long(loc: Loc, input: Value, max: usize) -> ErrorItem {
        let unit = if max == 1 { "character" } else { "characters" };
        ErrorItem::new("string_too_long", loc, format!("String should have at most {max} {unit}"), input)
            .with_ctx(json!({ "max_length": max }))
    }
    pub fn string_pattern_mismatch(loc: Loc, input: Value, pattern: &str) -> ErrorItem {
        ErrorItem::new("string_pattern_mismatch", loc, format!("String should match pattern '{pattern}'"), input)
            .with_ctx(json!({ "pattern": pattern }))
    }
    pub fn int_type(loc: Loc, input: Value) -> ErrorItem {
        ErrorItem::new("int_type", loc, "Input should be a valid integer", input)
    }
    pub fn int_parsing(loc: Loc, input: Value) -> ErrorItem {
        ErrorItem::new(
            "int_parsing",
            loc,
            "Input should be a valid integer, unable to parse string as an integer",
            input,
        )
    }
    pub fn int_from_float(loc: Loc, input: Value) -> ErrorItem {
        ErrorItem::new(
            "int_from_float",
            loc,
            "Input should be a valid integer, got a number with a fractional part",
            input,
        )
    }
    pub fn int_parsing_size(loc: Loc, input: Value) -> ErrorItem {
        ErrorItem::new(
            "int_parsing_size",
            loc,
            "Unable to parse input string as an integer, exceeded maximum size",
            input,
        )
    }
    pub fn float_type(loc: Loc, input: Value) -> ErrorItem {
        ErrorItem::new("float_type", loc, "Input should be a valid number", input)
    }
    pub fn float_parsing(loc: Loc, input: Value) -> ErrorItem {
        ErrorItem::new(
            "float_parsing",
            loc,
            "Input should be a valid number, unable to parse string as a number",
            input,
        )
    }
    pub fn bool_type(loc: Loc, input: Value) -> ErrorItem {
        ErrorItem::new("bool_type", loc, "Input should be a valid boolean", input)
    }
    pub fn bool_parsing(loc: Loc, input: Value) -> ErrorItem {
        ErrorItem::new("bool_parsing", loc, "Input should be a valid boolean, unable to interpret input", input)
    }
    pub fn decimal_type(loc: Loc, input: Value) -> ErrorItem {
        ErrorItem::new(
            "decimal_type",
            loc,
            "Decimal input should be an integer, float, string or Decimal object",
            input,
        )
    }
    pub fn decimal_parsing(loc: Loc, input: Value) -> ErrorItem {
        ErrorItem::new("decimal_parsing", loc, "Input should be a valid decimal", input)
    }
    pub fn finite_number(loc: Loc, input: Value) -> ErrorItem {
        ErrorItem::new("finite_number", loc, "Input should be a finite number", input)
    }
    pub fn decimal_max_digits(loc: Loc, input: Value, max: u64) -> ErrorItem {
        let unit = if max == 1 { "digit" } else { "digits" };
        ErrorItem::new(
            "decimal_max_digits",
            loc,
            format!("Decimal input should have no more than {max} {unit} in total"),
            input,
        )
        .with_ctx(json!({ "max_digits": max }))
    }
    pub fn decimal_max_places(loc: Loc, input: Value, places: u64) -> ErrorItem {
        let unit = if places == 1 { "decimal place" } else { "decimal places" };
        ErrorItem::new(
            "decimal_max_places",
            loc,
            format!("Decimal input should have no more than {places} {unit}"),
            input,
        )
        .with_ctx(json!({ "decimal_places": places }))
    }
    pub fn decimal_whole_digits(loc: Loc, input: Value, whole: u64) -> ErrorItem {
        let unit = if whole == 1 { "digit" } else { "digits" };
        ErrorItem::new(
            "decimal_whole_digits",
            loc,
            format!("Decimal input should have no more than {whole} {unit} before the decimal point"),
            input,
        )
        .with_ctx(json!({ "whole_digits": whole }))
    }
    pub fn greater_than(loc: Loc, input: Value, bound: &str, ctx: Value) -> ErrorItem {
        ErrorItem::new("greater_than", loc, format!("Input should be greater than {bound}"), input)
            .with_ctx(json!({ "gt": ctx }))
    }
    pub fn greater_than_equal(loc: Loc, input: Value, bound: &str, ctx: Value) -> ErrorItem {
        ErrorItem::new("greater_than_equal", loc, format!("Input should be greater than or equal to {bound}"), input)
            .with_ctx(json!({ "ge": ctx }))
    }
    pub fn less_than(loc: Loc, input: Value, bound: &str, ctx: Value) -> ErrorItem {
        ErrorItem::new("less_than", loc, format!("Input should be less than {bound}"), input)
            .with_ctx(json!({ "lt": ctx }))
    }
    pub fn less_than_equal(loc: Loc, input: Value, bound: &str, ctx: Value) -> ErrorItem {
        ErrorItem::new("less_than_equal", loc, format!("Input should be less than or equal to {bound}"), input)
            .with_ctx(json!({ "le": ctx }))
    }
    /// `'a', 'b' or 'c'` — pydantic's list of expected literals.
    pub fn expected_list(values: &[&str]) -> String {
        let quoted: Vec<String> = values.iter().map(|v| format!("'{v}'")).collect();
        match quoted.len() {
            0 => String::new(),
            1 => quoted[0].clone(),
            n => format!("{} or {}", quoted[..n - 1].join(", "), quoted[n - 1]),
        }
    }
    pub fn enum_(loc: Loc, input: Value, values: &[&str]) -> ErrorItem {
        let expected = expected_list(values);
        ErrorItem::new("enum", loc, format!("Input should be {expected}"), input)
            .with_ctx(json!({ "expected": expected }))
    }
    pub fn literal_error(loc: Loc, input: Value, values: &[&str]) -> ErrorItem {
        let expected = expected_list(values);
        ErrorItem::new("literal_error", loc, format!("Input should be {expected}"), input)
            .with_ctx(json!({ "expected": expected }))
    }
    pub fn list_type(loc: Loc, input: Value) -> ErrorItem {
        ErrorItem::new("list_type", loc, "Input should be a valid array", input)
    }
    pub fn date_from_datetime_parsing(loc: Loc, input: Value, error: &str) -> ErrorItem {
        ErrorItem::new(
            "date_from_datetime_parsing",
            loc,
            format!("Input should be a valid date or datetime, {error}"),
            input,
        )
        .with_ctx(json!({ "error": error }))
    }
    pub fn date_from_datetime_inexact(loc: Loc, input: Value) -> ErrorItem {
        ErrorItem::new(
            "date_from_datetime_inexact",
            loc,
            "Datetimes provided to dates should have zero time - e.g. be exact dates",
            input,
        )
    }
    pub fn date_type(loc: Loc, input: Value) -> ErrorItem {
        ErrorItem::new("date_type", loc, "Input should be a valid date", input)
    }
    /// A `ValueError` raised from a `field_validator`/`model_validator`.
    /// FastAPI's jsonable_encoder turns the exception object in `ctx` into `{}`.
    pub fn value_error(loc: Loc, input: Value, message: &str) -> ErrorItem {
        ErrorItem::new("value_error", loc, format!("Value error, {message}"), input).with_ctx(json!({ "error": {} }))
    }
}

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

/// Where the value came from. Decides the `input` of a `missing` error:
/// FastAPI echoes the whole body object for a missing body field but
/// `null` for a missing query/path parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Path,
    Query,
    Body,
}

impl Source {
    pub fn root(self) -> LocItem {
        LocItem::Key(
            match self {
                Source::Path => "path",
                Source::Query => "query",
                Source::Body => "body",
            }
            .to_string(),
        )
    }
}

/// A value being validated, with its location.
#[derive(Debug, Clone)]
pub struct Val<'a> {
    pub value: &'a Value,
    pub loc: Loc,
    pub source: Source,
}

/// A validated value that can be refined further (`min_len`, `ge`, ...).
#[derive(Debug, Clone)]
pub struct Checked<'a, T> {
    pub value: T,
    pub input: &'a Value,
    pub loc: Loc,
}

pub type Res<'a, T> = Result<Checked<'a, T>, ValidationErrors>;

/// A request DTO that can be validated from JSON-ish input.
pub trait FromInput: Sized {
    /// `true` only for `()`: "this route has no such input" (e.g. no body),
    /// so the extractor doesn't even read it.
    const ABSENT: bool = false;

    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors>;

    /// Validates a whole source (`{"name": ...}` body, query map, path map).
    fn from_source(value: &Value, source: Source) -> Result<Self, ValidationErrors> {
        Self::from_input(Val { value, loc: vec![source.root()], source })
    }
}

/// "Nothing to validate" — used for an absent path/query/body slot.
impl FromInput for () {
    const ABSENT: bool = true;

    fn from_input(_: Val<'_>) -> Result<Self, ValidationErrors> {
        Ok(())
    }
}

/// A Python enum (or `Literal`) that the validator can parse by value.
pub trait InputEnum: Sized + Copy + 'static {
    /// Every variant with its API value, in definition order.
    const VARIANTS: &'static [(Self, &'static str)];
}

/// PATCH-field tri-state, i.e. pydantic's `exclude_unset=True` view of an
/// `X | None = None` field: absent, explicitly `null`, or a value.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Patch<T> {
    #[default]
    Unset,
    Null,
    Value(T),
}

impl<T> Patch<T> {
    pub fn is_set(&self) -> bool {
        !matches!(self, Patch::Unset)
    }

    /// `None` when unset; `Some(None)` for null; `Some(Some(v))` for a value.
    pub fn into_option(self) -> Option<Option<T>> {
        match self {
            Patch::Unset => None,
            Patch::Null => Some(None),
            Patch::Value(v) => Some(Some(v)),
        }
    }

    pub fn as_value(&self) -> Option<&T> {
        match self {
            Patch::Value(v) => Some(v),
            _ => None,
        }
    }
}

/// Output slot of an [`Obj`] field. Holds a value once validation of that
/// field succeeded; [`Obj::finish`] returning `Ok` guarantees every slot is
/// filled, so `.value()` after it never panics.
#[derive(Debug)]
pub struct Out<T>(Option<T>);

impl<T> Out<T> {
    pub fn value(self) -> T {
        self.0.expect("Out::value() called before a successful Obj::finish()")
    }
}

fn child_loc(loc: &Loc, item: impl Into<LocItem>) -> Loc {
    let mut loc = loc.clone();
    loc.push(item.into());
    loc
}

impl<'a> Val<'a> {
    fn checked<T>(&self, value: T) -> Checked<'a, T> {
        Checked { value, input: self.value, loc: self.loc.clone() }
    }

    fn input(&self) -> Value {
        self.value.clone()
    }

    pub fn is_null(&self) -> bool {
        self.value.is_null()
    }

    /// Starts validating a model (pydantic `BaseModel`) at this position.
    pub fn object(&self) -> Result<Obj<'a>, ValidationErrors> {
        match self.value {
            Value::Object(map) => Ok(Obj {
                map,
                whole: self.value,
                loc: self.loc.clone(),
                source: self.source,
                errors: ValidationErrors::default(),
            }),
            _ => Err(err::model_attributes_type(self.loc.clone(), self.input()).into()),
        }
    }

    /// The error a `@model_validator(mode="after")` raising
    /// `ValueError(message)` produces: `value_error` at the model's own
    /// location, echoing the whole model input. Return it after
    /// `Obj::finish()` succeeded — pydantic only runs "after" validators
    /// once every field is valid.
    pub fn model_value_error(&self, message: &str) -> ValidationErrors {
        err::value_error(self.loc.clone(), self.input(), message).into()
    }

    /// A nested model field.
    pub fn model<T: FromInput>(&self) -> Res<'a, T> {
        T::from_input(self.clone()).map(|v| self.checked(v))
    }

    /// `str` (lax mode still refuses numbers/bools/null).
    pub fn str(&self) -> Res<'a, String> {
        match self.value {
            Value::String(s) => Ok(self.checked(s.clone())),
            _ => Err(err::string_type(self.loc.clone(), self.input()).into()),
        }
    }

    /// `int` with pydantic's lax coercions.
    pub fn int(&self) -> Res<'a, i64> {
        let loc = || self.loc.clone();
        let parsed = match self.value {
            Value::Bool(b) => Ok(i64::from(*b)),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Ok(i)
                } else if n.is_u64() {
                    Err(err::int_parsing_size(loc(), self.input()))
                } else {
                    let f = n.as_f64().unwrap_or(f64::NAN);
                    if f.fract() != 0.0 || !f.is_finite() {
                        Err(err::int_from_float(loc(), self.input()))
                    } else if f.abs() >= 9.2e18 {
                        Err(err::int_parsing_size(loc(), self.input()))
                    } else {
                        Ok(f as i64)
                    }
                }
            }
            Value::String(s) => parse_lax_int(s).ok_or_else(|| err::int_parsing(loc(), self.input())),
            _ => Err(err::int_type(loc(), self.input())),
        };
        parsed.map(|v| self.checked(v)).map_err(Into::into)
    }

    /// `float` with pydantic's lax coercions.
    pub fn float(&self) -> Res<'a, f64> {
        let parsed = match self.value {
            Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Value::Number(n) => Ok(n.as_f64().unwrap_or(f64::NAN)),
            Value::String(s) => {
                parse_py_float(s.trim()).ok_or_else(|| err::float_parsing(self.loc.clone(), self.input()))
            }
            _ => Err(err::float_type(self.loc.clone(), self.input())),
        };
        parsed.map(|v| self.checked(v)).map_err(Into::into)
    }

    /// `bool` with pydantic's lax coercions (`"yes"`, `"off"`, `1`, ...).
    pub fn bool(&self) -> Res<'a, bool> {
        let parsed = match self.value {
            Value::Bool(b) => Ok(*b),
            Value::Number(n) => {
                let f = n.as_f64().unwrap_or(f64::NAN);
                if f == 0.0 {
                    Ok(false)
                } else if f == 1.0 {
                    Ok(true)
                } else {
                    Err(err::bool_parsing(self.loc.clone(), self.input()))
                }
            }
            Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
                "0" | "off" | "f" | "false" | "n" | "no" => Ok(false),
                "1" | "on" | "t" | "true" | "y" | "yes" => Ok(true),
                _ => Err(err::bool_parsing(self.loc.clone(), self.input())),
            },
            _ => Err(err::bool_type(self.loc.clone(), self.input())),
        };
        parsed.map(|v| self.checked(v)).map_err(Into::into)
    }

    /// `Decimal` with pydantic's coercions (int, float via Rust's `f64`
    /// Display — as pydantic-core does — or string).
    pub fn decimal(&self) -> Res<'a, Decimal> {
        let loc = || self.loc.clone();
        let text = match self.value {
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    i.to_string()
                } else if let Some(u) = n.as_u64() {
                    u.to_string()
                } else {
                    let f = n.as_f64().unwrap_or(f64::NAN);
                    if !f.is_finite() {
                        return Err(err::finite_number(loc(), self.input()).into());
                    }
                    f.to_string()
                }
            }
            Value::String(s) => {
                let t = s.trim().to_ascii_lowercase();
                let t = t.trim_start_matches(['+', '-']);
                if matches!(t, "nan" | "snan" | "inf" | "infinity") {
                    return Err(err::finite_number(loc(), self.input()).into());
                }
                s.clone()
            }
            _ => return Err(err::decimal_type(loc(), self.input()).into()),
        };
        text.parse::<Decimal>().map(|d| self.checked(d)).map_err(|_| err::decimal_parsing(loc(), self.input()).into())
    }

    /// `datetime.date` as pydantic parses it (`YYYY-MM-DD`, or a datetime /
    /// unix timestamp with a zero time component).
    pub fn date(&self) -> Res<'a, NaiveDate> {
        let loc = || self.loc.clone();
        match self.value {
            Value::String(s) => parse_py_date(s).map(|d| self.checked(d)).map_err(|e| {
                match e {
                    DateError::Inexact => err::date_from_datetime_inexact(loc(), self.input()),
                    DateError::Parse(msg) => err::date_from_datetime_parsing(loc(), self.input(), msg),
                }
                .into()
            }),
            Value::Number(n) => {
                let secs = n.as_f64().unwrap_or(f64::NAN);
                if secs.fract() == 0.0 && (secs as i64).rem_euclid(86_400) == 0 {
                    let days = (secs as i64).div_euclid(86_400);
                    NaiveDate::from_ymd_opt(1970, 1, 1)
                        .and_then(|epoch| epoch.checked_add_signed(chrono::Duration::days(days)))
                        .map(|d| self.checked(d))
                        .ok_or_else(|| err::date_from_datetime_inexact(loc(), self.input()).into())
                } else {
                    Err(err::date_from_datetime_inexact(loc(), self.input()).into())
                }
            }
            _ => Err(err::date_type(loc(), self.input()).into()),
        }
    }

    /// A Python `str` enum, by value.
    pub fn enum_<E: InputEnum>(&self) -> Res<'a, E> {
        if let Value::String(s) = self.value
            && let Some((variant, _)) = E::VARIANTS.iter().find(|(_, v)| v == s)
        {
            return Ok(self.checked(*variant));
        }
        let values: Vec<&str> = E::VARIANTS.iter().map(|(_, v)| *v).collect();
        Err(err::enum_(self.loc.clone(), self.input(), &values).into())
    }

    /// A `Literal["a", "b"]` field; returns the matched literal.
    pub fn literal(&self, values: &[&'static str]) -> Res<'a, &'static str> {
        if let Value::String(s) = self.value
            && let Some(v) = values.iter().find(|v| *v == s)
        {
            return Ok(self.checked(*v));
        }
        Err(err::literal_error(self.loc.clone(), self.input(), values).into())
    }

    /// `list[T]`: every item is validated, errors are collected with the index in `loc`.
    pub fn list<T>(&self, item: impl Fn(Val<'a>) -> Res<'a, T>) -> Res<'a, Vec<T>> {
        let Value::Array(items) = self.value else {
            return Err(err::list_type(self.loc.clone(), self.input()).into());
        };
        let mut out = Vec::with_capacity(items.len());
        let mut errors = ValidationErrors::default();
        for (i, value) in items.iter().enumerate() {
            match item(Val { value, loc: child_loc(&self.loc, i), source: self.source }) {
                Ok(c) => out.push(c.value),
                Err(e) => errors.extend(e),
            }
        }
        if errors.is_empty() { Ok(self.checked(out)) } else { Err(errors) }
    }
}

impl<'a, T> Checked<'a, T> {
    fn fail(&self, f: impl FnOnce(Loc, Value) -> ErrorItem) -> ValidationErrors {
        f(self.loc.clone(), self.input.clone()).into()
    }

    /// Transform the value (what a pydantic `field_validator` returning a
    /// new value does, e.g. `.upper()`).
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Checked<'a, U> {
        Checked { value: f(self.value), input: self.input, loc: self.loc }
    }

    /// A `field_validator` that may raise `ValueError(message)`.
    pub fn check(self, f: impl FnOnce(&T) -> Result<(), String>) -> Res<'a, T> {
        match f(&self.value) {
            Ok(()) => Ok(self),
            Err(message) => Err(self.fail(|loc, input| err::value_error(loc, input, &message))),
        }
    }
}

impl<'a> Checked<'a, String> {
    pub fn min_len(self, min: usize) -> Res<'a, String> {
        if self.value.chars().count() < min {
            return Err(self.fail(|l, i| err::string_too_short(l, i, min)));
        }
        Ok(self)
    }

    pub fn max_len(self, max: usize) -> Res<'a, String> {
        if self.value.chars().count() > max {
            return Err(self.fail(|l, i| err::string_too_long(l, i, max)));
        }
        Ok(self)
    }

    /// `Field(pattern=...)` — pydantic uses Rust's `regex` crate too, with
    /// `search` semantics (anchors come from the pattern itself).
    pub fn pattern(self, pattern: &'static str) -> Res<'a, String> {
        if !cached_regex(pattern).is_match(&self.value) {
            return Err(self.fail(|l, i| err::string_pattern_mismatch(l, i, pattern)));
        }
        Ok(self)
    }
}

macro_rules! numeric_bounds {
    ($ty:ty, $fmt:expr, $ctx:expr) => {
        impl<'a> Checked<'a, $ty> {
            pub fn ge(self, bound: $ty) -> Res<'a, $ty> {
                if self.value >= bound {
                    Ok(self)
                } else {
                    Err(self.fail(|l, i| err::greater_than_equal(l, i, &$fmt(&bound), $ctx(&bound))))
                }
            }
            pub fn gt(self, bound: $ty) -> Res<'a, $ty> {
                if self.value > bound {
                    Ok(self)
                } else {
                    Err(self.fail(|l, i| err::greater_than(l, i, &$fmt(&bound), $ctx(&bound))))
                }
            }
            pub fn le(self, bound: $ty) -> Res<'a, $ty> {
                if self.value <= bound {
                    Ok(self)
                } else {
                    Err(self.fail(|l, i| err::less_than_equal(l, i, &$fmt(&bound), $ctx(&bound))))
                }
            }
            pub fn lt(self, bound: $ty) -> Res<'a, $ty> {
                if self.value < bound {
                    Ok(self)
                } else {
                    Err(self.fail(|l, i| err::less_than(l, i, &$fmt(&bound), $ctx(&bound))))
                }
            }
        }
    };
}

numeric_bounds!(i64, |b: &i64| b.to_string(), |b: &i64| Value::from(*b));
numeric_bounds!(f64, |b: &f64| py_float_repr(*b), |b: &f64| serde_json::json!(*b));

impl<'a> Checked<'a, Decimal> {
    /// `max_digits`/`decimal_places`, checked like pydantic-core (on the
    /// normalised value, so `"1.000"` passes `decimal_places=2`).
    pub fn digits(self, max_digits: Option<u64>, decimal_places: Option<u64>) -> Res<'a, Decimal> {
        let normalized = self.value.normalize();
        let (coefficient, exponent) = normalized.parts();
        let len = coefficient.magnitude().to_str_radix(10).len() as u64;
        let (digits, decimals) = if exponent >= 0 {
            (len + exponent as u64, 0)
        } else {
            let decimals = (-exponent) as u64;
            (len.max(decimals), decimals)
        };
        if let Some(max) = max_digits
            && digits > max
        {
            return Err(self.fail(|l, i| err::decimal_max_digits(l, i, max)));
        }
        if let Some(places) = decimal_places {
            if decimals > places {
                return Err(self.fail(|l, i| err::decimal_max_places(l, i, places)));
            }
            if let Some(max) = max_digits {
                let whole = max.saturating_sub(places);
                if digits.saturating_sub(decimals) > whole {
                    return Err(self.fail(|l, i| err::decimal_whole_digits(l, i, whole)));
                }
            }
        }
        Ok(self)
    }

    pub fn ge(self, bound: &str) -> Res<'a, Decimal> {
        let b: Decimal = bound.parse().expect("valid decimal bound");
        if self.value >= b {
            Ok(self)
        } else {
            Err(self.fail(|l, i| err::greater_than_equal(l, i, &b.to_string(), err::decimal_ctx(&b))))
        }
    }
    pub fn gt(self, bound: &str) -> Res<'a, Decimal> {
        let b: Decimal = bound.parse().expect("valid decimal bound");
        if self.value > b {
            Ok(self)
        } else {
            Err(self.fail(|l, i| err::greater_than(l, i, &b.to_string(), err::decimal_ctx(&b))))
        }
    }
    pub fn le(self, bound: &str) -> Res<'a, Decimal> {
        let b: Decimal = bound.parse().expect("valid decimal bound");
        if self.value <= b {
            Ok(self)
        } else {
            Err(self.fail(|l, i| err::less_than_equal(l, i, &b.to_string(), err::decimal_ctx(&b))))
        }
    }
    pub fn lt(self, bound: &str) -> Res<'a, Decimal> {
        let b: Decimal = bound.parse().expect("valid decimal bound");
        if self.value < b {
            Ok(self)
        } else {
            Err(self.fail(|l, i| err::less_than(l, i, &b.to_string(), err::decimal_ctx(&b))))
        }
    }
}

// ---------------------------------------------------------------------------
// Model (object) reader
// ---------------------------------------------------------------------------

/// Reads the fields of one model. Call the field methods **in the same
/// order as the pydantic model declares its fields** — that is the order
/// FastAPI lists errors in. Errors are collected, not short-circuited.
pub struct Obj<'a> {
    map: &'a Map<String, Value>,
    whole: &'a Value,
    loc: Loc,
    source: Source,
    errors: ValidationErrors,
}

impl<'a> Obj<'a> {
    fn val(&self, key: &str, value: &'a Value) -> Val<'a> {
        Val { value, loc: child_loc(&self.loc, key), source: self.source }
    }

    fn run<T>(&mut self, v: Val<'a>, f: impl FnOnce(Val<'a>) -> Res<'a, T>) -> Option<T> {
        match f(v) {
            Ok(c) => Some(c.value),
            Err(e) => {
                self.errors.extend(e);
                None
            }
        }
    }

    fn missing(&mut self, key: &str) {
        let input = match self.source {
            Source::Body => self.whole.clone(),
            Source::Path | Source::Query => Value::Null,
        };
        self.errors.0.push(err::missing(child_loc(&self.loc, key), input));
    }

    /// Raw access for unusual cases.
    pub fn get(&self, key: &str) -> Option<&'a Value> {
        self.map.get(key)
    }

    /// `field: T` (no default): absent → `missing`.
    pub fn required<T>(&mut self, key: &str, f: impl FnOnce(Val<'a>) -> Res<'a, T>) -> Out<T> {
        match self.map.get(key) {
            None => {
                self.missing(key);
                Out(None)
            }
            Some(value) => {
                let v = self.val(key, value);
                Out(self.run(v, f))
            }
        }
    }

    /// `field: T | None` (no default): absent → `missing`, null → `None`.
    pub fn required_nullable<T>(&mut self, key: &str, f: impl FnOnce(Val<'a>) -> Res<'a, T>) -> Out<Option<T>> {
        match self.map.get(key) {
            None => {
                self.missing(key);
                Out(None)
            }
            Some(Value::Null) => Out(Some(None)),
            Some(value) => {
                let v = self.val(key, value);
                Out(self.run(v, f).map(Some))
            }
        }
    }

    /// `field: T | None = None`: absent or null → `None`.
    pub fn optional<T>(&mut self, key: &str, f: impl FnOnce(Val<'a>) -> Res<'a, T>) -> Out<Option<T>> {
        match self.map.get(key) {
            None | Some(Value::Null) => Out(Some(None)),
            Some(value) => {
                let v = self.val(key, value);
                Out(self.run(v, f).map(Some))
            }
        }
    }

    /// `field: T = default`: absent → default; null is validated (and
    /// therefore rejected by non-nullable validators), like pydantic.
    pub fn with_default<T>(
        &mut self,
        key: &str,
        default: impl FnOnce() -> T,
        f: impl FnOnce(Val<'a>) -> Res<'a, T>,
    ) -> Out<T> {
        match self.map.get(key) {
            None => Out(Some(default())),
            Some(value) => {
                let v = self.val(key, value);
                Out(self.run(v, f))
            }
        }
    }

    /// `field: T | None = None` on a PATCH model read with `exclude_unset`.
    pub fn patch<T>(&mut self, key: &str, f: impl FnOnce(Val<'a>) -> Res<'a, T>) -> Out<Patch<T>> {
        match self.map.get(key) {
            None => Out(Some(Patch::Unset)),
            Some(Value::Null) => Out(Some(Patch::Null)),
            Some(value) => {
                let v = self.val(key, value);
                Out(self.run(v, f).map(Patch::Value))
            }
        }
    }

    /// Ends field validation; `Err` carries every collected error.
    pub fn finish(self) -> Result<(), ValidationErrors> {
        if self.errors.is_empty() { Ok(()) } else { Err(self.errors) }
    }
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

fn cached_regex(pattern: &'static str) -> Regex {
    static CACHE: LazyLock<Mutex<HashMap<&'static str, Regex>>> = LazyLock::new(Default::default);
    let mut cache = CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.entry(pattern).or_insert_with(|| Regex::new(pattern).expect("validation pattern must compile")).clone()
}

/// pydantic's str → int: surrounding whitespace and `_` separators are
/// fine, so is a float literal with a zero fraction (`"5.0"`).
fn parse_lax_int(s: &str) -> Option<i64> {
    let t = s.trim();
    if t.is_empty() || t.starts_with('_') || t.ends_with('_') || t.contains("__") {
        return None;
    }
    let t = t.replace('_', "");
    if let Ok(i) = t.parse::<i64>() {
        return Some(i);
    }
    let (int_part, frac) = t.split_once('.')?;
    if !frac.chars().all(|c| c == '0') {
        return None;
    }
    let digits = int_part.trim_start_matches(['+', '-']);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    int_part.parse().ok()
}

fn parse_py_float(s: &str) -> Option<f64> {
    let lower = s.to_ascii_lowercase();
    match lower.trim_start_matches(['+', '-']) {
        "inf" | "infinity" | "nan" => {}
        _ if s.bytes().any(|b| b.is_ascii_alphabetic() && b != b'e' && b != b'E') => return None,
        _ => {}
    }
    s.replace('_', "").parse().ok()
}

enum DateError {
    Inexact,
    Parse(&'static str),
}

/// Subset of speedate's date parsing with pydantic's error wording.
fn parse_py_date(s: &str) -> Result<NaiveDate, DateError> {
    let b = s.as_bytes();
    if !b.is_empty() && b.iter().all(u8::is_ascii_digit) {
        // A bare number is a unix timestamp; only midnight timestamps are dates.
        return match s.parse::<i64>() {
            Ok(secs) if secs.rem_euclid(86_400) == 0 => NaiveDate::from_ymd_opt(1970, 1, 1)
                .and_then(|epoch| epoch.checked_add_signed(chrono::Duration::days(secs.div_euclid(86_400))))
                .ok_or(DateError::Inexact),
            _ => Err(DateError::Inexact),
        };
    }
    if b.len() < 10 {
        return Err(DateError::Parse("input is too short"));
    }
    if !b[..4].iter().all(u8::is_ascii_digit) {
        return Err(DateError::Parse("invalid character in year"));
    }
    if b[4] != b'-' || b[7] != b'-' {
        return Err(DateError::Parse("invalid date separator, expected `-`"));
    }
    if !b[5..7].iter().all(u8::is_ascii_digit) {
        return Err(DateError::Parse("invalid character in month"));
    }
    if !b[8..10].iter().all(u8::is_ascii_digit) {
        return Err(DateError::Parse("invalid character in day"));
    }
    let year: i32 = s[..4].parse().map_err(|_| DateError::Parse("invalid character in year"))?;
    let month: u32 = s[5..7].parse().map_err(|_| DateError::Parse("invalid character in month"))?;
    let day: u32 = s[8..10].parse().map_err(|_| DateError::Parse("invalid character in day"))?;
    if !(1..=12).contains(&month) {
        return Err(DateError::Parse("month value is outside expected range of 1-12"));
    }
    let date =
        NaiveDate::from_ymd_opt(year, month, day).ok_or(DateError::Parse("day value is outside expected range"))?;
    if b.len() == 10 {
        return Ok(date);
    }
    // A full datetime is accepted only when its time is exactly midnight.
    let rest = &s[10..];
    if !rest.starts_with(['T', 't', ' ', '_']) {
        return Err(DateError::Parse("unexpected extra characters at the end of the input"));
    }
    let candidate = format!("{}T{}", &s[..10], &rest[1..]);
    let naive = candidate
        .trim_end_matches('Z')
        .parse::<NaiveDateTime>()
        .or_else(|_| chrono::DateTime::parse_from_rfc3339(&candidate).map(|d| d.naive_local()))
        .map_err(|_| DateError::Parse("invalid character in hour"))?;
    if naive.time() == chrono::NaiveTime::MIN { Ok(date) } else { Err(DateError::Inexact) }
}

/// Clients serialise a `Patch` as the field it stands for; pair it with
/// `#[serde(skip_serializing_if = "Patch::is_unset")]` so `Unset` is omitted.
impl<T: Serialize> Serialize for Patch<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Patch::Unset | Patch::Null => serializer.serialize_none(),
            Patch::Value(v) => v.serialize(serializer),
        }
    }
}

impl<T> Patch<T> {
    pub fn is_unset(&self) -> bool {
        matches!(self, Patch::Unset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[allow(dead_code)]
    struct M {
        name: String,
        n: Option<i64>,
        amount: Patch<Decimal>,
    }

    impl FromInput for M {
        fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
            let mut o = v.object()?;
            let name = o.required("name", |v| v.str()?.min_len(1)?.max_len(5));
            let n = o.optional("n", |v| v.int()?.ge(1)?.le(24));
            let amount = o.patch("amount", |v| v.decimal()?.digits(Some(14), Some(2))?.ge("0"));
            o.finish()?;
            Ok(Self { name: name.value(), n: n.value(), amount: amount.value() })
        }
    }

    fn errors(body: Value) -> String {
        match M::from_source(&body, Source::Body) {
            Ok(_) => "ok".into(),
            Err(e) => serde_json::to_string(&e.0).unwrap(),
        }
    }

    #[test]
    fn reproduces_pydantic_errors() {
        assert_eq!(
            errors(json!({})),
            r#"[{"type":"missing","loc":["body","name"],"msg":"Field required","input":{}}]"#
        );
        assert_eq!(
            errors(json!({"name": "", "n": 0, "amount": 12.345})),
            concat!(
                r#"[{"type":"string_too_short","loc":["body","name"],"msg":"String should have at least 1 character","input":"","ctx":{"min_length":1}},"#,
                r#"{"type":"greater_than_equal","loc":["body","n"],"msg":"Input should be greater than or equal to 1","input":0,"ctx":{"ge":1}},"#,
                r#"{"type":"decimal_max_places","loc":["body","amount"],"msg":"Decimal input should have no more than 2 decimal places","input":12.345,"ctx":{"decimal_places":2}}]"#
            )
        );
        assert_eq!(
            errors(json!([1])),
            r#"[{"type":"model_attributes_type","loc":["body"],"msg":"Input should be a valid dictionary or object to extract fields from","input":[1]}]"#
        );
        assert_eq!(
            errors(json!({"name": 5, "n": 1.5})),
            concat!(
                r#"[{"type":"string_type","loc":["body","name"],"msg":"Input should be a valid string","input":5},"#,
                r#"{"type":"int_from_float","loc":["body","n"],"msg":"Input should be a valid integer, got a number with a fractional part","input":1.5}]"#
            )
        );
        assert_eq!(errors(json!({"name": "ok", "n": "7.0", "amount": 1e3})), "ok");
    }

    #[test]
    fn patch_tristate_and_decimal_repr() {
        let m = M::from_source(&json!({"name": "a", "amount": "100.50"}), Source::Body).ok().unwrap();
        assert_eq!(m.amount.as_value().unwrap().to_string(), "100.50");
        assert_eq!(m.n, None);
        let m = M::from_source(&json!({"name": "a", "amount": null}), Source::Body).ok().unwrap();
        assert_eq!(m.amount, Patch::Null);
        let m = M::from_source(&json!({"name": "a", "amount": 100.0}), Source::Body).ok().unwrap();
        assert_eq!(m.amount.as_value().unwrap().to_string(), "100");
    }

    #[test]
    fn expected_lists() {
        assert_eq!(err::expected_list(&["income", "expense"]), "'income' or 'expense'");
        assert_eq!(err::expected_list(&["low", "medium", "high"]), "'low', 'medium' or 'high'");
    }

    #[test]
    fn dates() {
        let v = json!("2026-13-01");
        let r = Val { value: &v, loc: vec![], source: Source::Body }.date();
        assert_eq!(
            r.unwrap_err().0[0].msg,
            "Input should be a valid date or datetime, month value is outside expected range of 1-12"
        );
        let v = json!("2026-01-01T00:00:00");
        assert!(Val { value: &v, loc: vec![], source: Source::Body }.date().is_ok());
    }
}
