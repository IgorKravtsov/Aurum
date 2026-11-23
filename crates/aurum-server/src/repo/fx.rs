//! SQL for the `fx_rates` cache (port of the queries in the former Python backend's `app/services/fx_service.py`).

use aurum_domain::Decimal;
use aurum_domain::fx::FxRateRow;
use chrono::NaiveDate;
use sqlx::PgExecutor;

/// Every cached row that can answer one of `requested_dates` directly, or
/// serve as an outage fallback (a publication within the window).
pub async fn cached(
    db: impl PgExecutor<'_>,
    currencies: &[String],
    requested_dates: &[NaiveDate],
    earliest: NaiveDate,
    latest: NaiveDate,
) -> sqlx::Result<Vec<FxRateRow>> {
    sqlx::query_as(
        "SELECT currency, requested_date, effective_date, rate_to_pln, fetched_at FROM fx_rates \
         WHERE fx_rates.currency = ANY($1) \
         AND (fx_rates.requested_date = ANY($2) OR fx_rates.effective_date BETWEEN $3 AND $4)",
    )
    .bind(currencies)
    .bind(requested_dates)
    .bind(earliest)
    .bind(latest)
    .fetch_all(db)
    .await
}

/// One freshly fetched rate to persist.
#[derive(Debug, Clone, PartialEq)]
pub struct NewFxRate {
    pub currency: String,
    pub requested_date: NaiveDate,
    pub effective_date: NaiveDate,
    pub rate_to_pln: Decimal,
}

/// Inserts the rows in order (one statement, like SQLAlchemy's
/// `insert().values([...])`), refreshing an existing `(currency,
/// requested_date)` row instead — `provider` has an ORM-side default only.
pub async fn upsert(db: impl PgExecutor<'_>, rows: &[NewFxRate]) -> sqlx::Result<()> {
    let currencies: Vec<&str> = rows.iter().map(|r| r.currency.as_str()).collect();
    let requested: Vec<NaiveDate> = rows.iter().map(|r| r.requested_date).collect();
    let effective: Vec<NaiveDate> = rows.iter().map(|r| r.effective_date).collect();
    let rates: Vec<Decimal> = rows.iter().map(|r| r.rate_to_pln.clone()).collect();
    sqlx::query(
        "INSERT INTO fx_rates (currency, requested_date, effective_date, rate_to_pln, provider) \
         SELECT currency, requested_date, effective_date, rate_to_pln, 'nbp' \
         FROM unnest($1::varchar[], $2::date[], $3::date[], $4::numeric[]) WITH ORDINALITY \
              AS new(currency, requested_date, effective_date, rate_to_pln, n) \
         ORDER BY n \
         ON CONFLICT ON CONSTRAINT uq_fx_rates_currency_requested_date DO UPDATE SET \
           effective_date = excluded.effective_date, rate_to_pln = excluded.rate_to_pln, \
           provider = excluded.provider, fetched_at = now()",
    )
    .bind(currencies)
    .bind(requested)
    .bind(effective)
    .bind(rates)
    .execute(db)
    .await?;
    Ok(())
}
