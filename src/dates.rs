//! Dates (decision 9). Search Console's days are Pacific Time days; the CLI
//! sends `YYYY-MM-DD` and never a time.

use time::format_description::BorrowedFormatItem;
use time::macros::format_description;
use time::{Date, Duration, Month, OffsetDateTime};

const FORMAT: &[BorrowedFormatItem<'_>] = format_description!("[year]-[month]-[day]");

/// How long Search Console keeps performance data.
pub const RETENTION_MONTHS: i32 = 16;

/// Parses `YYYY-MM-DD`, for clap.
///
/// # Errors
///
/// The text for clap.
pub fn parse(value: &str) -> Result<Date, String> {
    Date::parse(value, FORMAT).map_err(|_| format!("{value:?} is not a date: write YYYY-MM-DD"))
}

/// `YYYY-MM-DD`.
#[must_use]
pub fn format(date: Date) -> String {
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

/// Today in UTC. Pacific Time is never ahead of UTC, so this is today or
/// tomorrow in Search Console's time zone.
#[must_use]
pub fn today() -> Date {
    OffsetDateTime::now_utc().date()
}

/// `date` minus `days` days, or the earliest date when that is out of range.
#[must_use]
pub fn minus(date: Date, days: i64) -> Date {
    date.checked_sub(Duration::days(days)).unwrap_or(Date::MIN)
}

/// `date` plus `days` days, or the latest date when that is out of range.
#[must_use]
pub fn plus(date: Date, days: i64) -> Date {
    date.checked_add(Duration::days(days)).unwrap_or(Date::MAX)
}

/// The number of days from `start` to `end`, both included.
#[must_use]
pub fn length(start: Date, end: Date) -> i64 {
    (end - start).whole_days() + 1
}

/// The same day `months` months earlier, or the month's last day when it is
/// shorter.
#[must_use]
pub fn months_before(date: Date, months: i32) -> Date {
    let total = date.year() * 12 + i32::from(u8::from(date.month())) - 1 - months;
    let year = total.div_euclid(12);
    let month = u8::try_from(total.rem_euclid(12) + 1)
        .ok()
        .and_then(|m| Month::try_from(m).ok())
        .unwrap_or(Month::January);
    let day = date.day().min(month.length(year));
    Date::from_calendar_date(year, month, day).unwrap_or(Date::MIN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;

    #[test]
    fn only_a_calendar_date_parses() {
        assert_eq!(parse("2026-02-28").unwrap(), date!(2026 - 02 - 28));
        for bad in [
            "2026-02-30",
            "2026-2-1",
            "26-01-01",
            "2026-01-01T00:00:00",
            "",
        ] {
            assert!(parse(bad).unwrap_err().contains("YYYY-MM-DD"), "{bad}");
        }
    }

    #[test]
    fn format_pads() {
        assert_eq!(format(date!(0987 - 01 - 02)), "0987-01-02");
    }

    #[test]
    fn day_arithmetic() {
        assert_eq!(minus(date!(2026 - 03 - 01), 1), date!(2026 - 02 - 28));
        assert_eq!(plus(date!(2026 - 02 - 28), 1), date!(2026 - 03 - 01));
        assert_eq!(length(date!(2026 - 01 - 01), date!(2026 - 01 - 28)), 28);
        assert_eq!(minus(Date::MIN, 1), Date::MIN);
        assert_eq!(plus(Date::MAX, 1), Date::MAX);
        assert!(today() > date!(2026 - 01 - 01));
    }

    #[test]
    fn months_before_clamps_to_the_month_end() {
        assert_eq!(
            months_before(date!(2026 - 11 - 08), 16),
            date!(2025 - 07 - 08)
        );
        assert_eq!(
            months_before(date!(2026 - 06 - 30), 16),
            date!(2025 - 02 - 28)
        );
        assert_eq!(
            months_before(date!(2026 - 01 - 31), 1),
            date!(2025 - 12 - 31)
        );
        assert_eq!(
            months_before(date!(2028 - 04 - 29), 2),
            date!(2028 - 02 - 29)
        );
    }
}
