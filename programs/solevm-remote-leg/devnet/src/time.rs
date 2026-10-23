//! UTC timestamps without a date crate.

use std::time::{SystemTime, UNIX_EPOCH};

/// ISO 8601 UTC text for a unix timestamp.
pub(crate) fn utc(timestamp: i64) -> String {
    civil(timestamp).unwrap_or_else(|| format!("unix:{timestamp}"))
}

pub(crate) fn now_utc() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    utc(i64::try_from(seconds).unwrap_or(i64::MAX))
}

/// Howard Hinnant's days-to-civil conversion, with every step checked.
fn civil(timestamp: i64) -> Option<String> {
    let days = timestamp.checked_div_euclid(86_400)?;
    let seconds = timestamp.checked_rem_euclid(86_400)?;
    let shifted = days.checked_add(719_468)?;
    let era = shifted.checked_div_euclid(146_097)?;
    let day_of_era = shifted.checked_sub(era.checked_mul(146_097)?)?;
    let year_of_era = day_of_era
        .checked_sub(day_of_era.checked_div(1_460)?)?
        .checked_add(day_of_era.checked_div(36_524)?)?
        .checked_sub(day_of_era.checked_div(146_096)?)?
        .checked_div(365)?;
    let day_of_year = day_of_era.checked_sub(
        year_of_era
            .checked_mul(365)?
            .checked_add(year_of_era.checked_div(4)?)?
            .checked_sub(year_of_era.checked_div(100)?)?,
    )?;
    let month_index = day_of_year
        .checked_mul(5)?
        .checked_add(2)?
        .checked_div(153)?;
    let day = day_of_year
        .checked_sub(
            month_index
                .checked_mul(153)?
                .checked_add(2)?
                .checked_div(5)?,
        )?
        .checked_add(1)?;
    let month = if month_index < 10 {
        month_index.checked_add(3)?
    } else {
        month_index.checked_sub(9)?
    };
    let year = year_of_era
        .checked_add(era.checked_mul(400)?)?
        .checked_add(i64::from(month <= 2))?;
    let hour = seconds.checked_div(3_600)?;
    let minute = seconds.checked_rem(3_600)?.checked_div(60)?;
    let second = seconds.checked_rem(60)?;
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    ))
}

#[cfg(test)]
mod tests {
    use super::utc;

    #[test]
    fn known_instants_format_exactly() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(utc(1_792_684_799), "2026-10-22T15:59:59Z");
    }
}
