//! Shared enum types (port of the former Python backend's `app/models/enums.py`).
//!
//! Two spellings per variant, and mixing them up is the classic porting bug:
//! * the **API value** (`"expense"`) — what JSON carries, what pydantic
//!   validates, what `Serialize`/`Deserialize` use;
//! * the **DB name** (`"EXPENSE"`) — SQLAlchemy's non-native `Enum` column
//!   stores the Python member *name*, so that is what Postgres holds. The
//!   `sqlx` impls generated here bind/decode the DB name, so in SQL always
//!   bind the enum itself (`.bind(kind)`), never `kind.as_str()`.

/// Declares a Python-style `str` enum. See the module docs.
#[macro_export]
macro_rules! py_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident { $( $(#[$vmeta:meta])* $variant:ident = ($value:literal, $db:literal) ),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name { $( $(#[$vmeta])* $variant ),+ }

        impl $name {
            /// Every variant in Python definition order.
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            /// The API value (`.value` in Python).
            pub const fn as_str(self) -> &'static str {
                match self { $($name::$variant => $value),+ }
            }

            /// The stored DB value (`.name` in Python).
            pub const fn db_name(self) -> &'static str {
                match self { $($name::$variant => $db),+ }
            }

            pub fn from_value(value: &str) -> Option<Self> {
                match value { $($value => Some($name::$variant),)+ _ => None }
            }

            pub fn from_db_name(name: &str) -> Option<Self> {
                match name { $($db => Some($name::$variant),)+ _ => None }
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl $crate::validation::InputEnum for $name {
            const VARIANTS: &'static [(Self, &'static str)] = &[$(($name::$variant, $value)),+];
        }

        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = <String as ::serde::Deserialize>::deserialize(d)?;
                Self::from_value(&s).ok_or_else(|| ::serde::de::Error::custom(format!(
                    "unknown {} value {s:?}", stringify!($name)
                )))
            }
        }

        $crate::__py_enum_sqlx!($name);
        $crate::__py_enum_openapi!($name, [$($value),+]);
    };
}

#[cfg(feature = "sqlx")]
#[doc(hidden)]
#[macro_export]
macro_rules! __py_enum_sqlx {
    ($name:ident) => {
        impl ::sqlx::Type<::sqlx::Postgres> for $name {
            fn type_info() -> ::sqlx::postgres::PgTypeInfo {
                <String as ::sqlx::Type<::sqlx::Postgres>>::type_info()
            }
            fn compatible(ty: &::sqlx::postgres::PgTypeInfo) -> bool {
                <String as ::sqlx::Type<::sqlx::Postgres>>::compatible(ty)
            }
        }

        impl<'r> ::sqlx::Decode<'r, ::sqlx::Postgres> for $name {
            fn decode(value: ::sqlx::postgres::PgValueRef<'r>) -> Result<Self, ::sqlx::error::BoxDynError> {
                let s = <&str as ::sqlx::Decode<::sqlx::Postgres>>::decode(value)?;
                $name::from_db_name(s).ok_or_else(|| format!("unknown {} DB name {s:?}", stringify!($name)).into())
            }
        }

        impl ::sqlx::Encode<'_, ::sqlx::Postgres> for $name {
            fn encode_by_ref(
                &self,
                buf: &mut ::sqlx::postgres::PgArgumentBuffer,
            ) -> Result<::sqlx::encode::IsNull, ::sqlx::error::BoxDynError> {
                <&str as ::sqlx::Encode<::sqlx::Postgres>>::encode_by_ref(&self.db_name(), buf)
            }
        }
    };
}

#[cfg(not(feature = "sqlx"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __py_enum_sqlx {
    ($name:ident) => {};
}

#[cfg(feature = "openapi")]
#[doc(hidden)]
#[macro_export]
macro_rules! __py_enum_openapi {
    ($name:ident, [$($value:literal),+]) => {
        impl ::utoipa::PartialSchema for $name {
            fn schema() -> ::utoipa::openapi::RefOr<::utoipa::openapi::schema::Schema> {
                ::utoipa::openapi::schema::ObjectBuilder::new()
                    .schema_type(::utoipa::openapi::schema::Type::String)
                    .enum_values(Some([$($value),+]))
                    .into()
            }
        }
        impl ::utoipa::ToSchema for $name {}
    };
}

#[cfg(not(feature = "openapi"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __py_enum_openapi {
    ($name:ident, [$($value:literal),+]) => {};
}

py_enum! {
    pub enum AccountType {
        Checking = ("checking", "CHECKING"),
        DebitCard = ("debit_card", "DEBIT_CARD"),
        Savings = ("savings", "SAVINGS"),
        CreditCard = ("credit_card", "CREDIT_CARD"),
        Cash = ("cash", "CASH"),
        Investment = ("investment", "INVESTMENT"),
        Other = ("other", "OTHER"),
    }
}

py_enum! {
    pub enum CategoryKind {
        Income = ("income", "INCOME"),
        Expense = ("expense", "EXPENSE"),
    }
}

py_enum! {
    pub enum TransactionType {
        Income = ("income", "INCOME"),
        Expense = ("expense", "EXPENSE"),
        Transfer = ("transfer", "TRANSFER"),
    }
}

py_enum! {
    /// Net-worth categories tracked manually (Cash is derived from Account
    /// balances instead — see the net_worth service).
    pub enum AssetClass {
        Investments = ("investments", "INVESTMENTS"),
        Crypto = ("crypto", "CRYPTO"),
        RealEstate = ("real_estate", "REAL_ESTATE"),
        Vehicles = ("vehicles", "VEHICLES"),
        PreciousMetals = ("precious_metals", "PRECIOUS_METALS"),
        Other = ("other", "OTHER"),
    }
}

py_enum! {
    /// How an asset behaves month to month — set by the user, not inferred.
    pub enum CapitalRole {
        /// e.g. a rented-out apartment
        Income = ("income", "INCOME"),
        /// e.g. a laptop used for work, furniture
        Neutral = ("neutral", "NEUTRAL"),
        /// e.g. a personal car, an idle depreciating gadget
        Drain = ("drain", "DRAIN"),
    }
}

py_enum! {
    pub enum RecurringFrequency {
        Weekly = ("weekly", "WEEKLY"),
        Monthly = ("monthly", "MONTHLY"),
        Yearly = ("yearly", "YEARLY"),
    }
}

py_enum! {
    pub enum CryptoTransactionType {
        Buy = ("buy", "BUY"),
        Sell = ("sell", "SELL"),
    }
}

py_enum! {
    /// Risk of loss, not asset class — set by the user. Cash is always LOW.
    pub enum RiskLevel {
        Low = ("low", "LOW"),
        Medium = ("medium", "MEDIUM"),
        High = ("high", "HIGH"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_value_and_db_name_differ() {
        assert_eq!(CategoryKind::Expense.as_str(), "expense");
        assert_eq!(CategoryKind::Expense.db_name(), "EXPENSE");
        assert_eq!(serde_json::to_string(&AccountType::DebitCard).unwrap(), "\"debit_card\"");
        assert_eq!(AssetClass::from_db_name("REAL_ESTATE"), Some(AssetClass::RealEstate));
    }
}
