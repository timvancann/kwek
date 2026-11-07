//! Timestamps. Google sends RFC 3339 with a varying number of fractional digits, so
//! string comparison is wrong; everything is stored as integer microseconds since the epoch.

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub type Micros = i64;

pub fn parse(s: &str) -> Option<Micros> {
    let t = OffsetDateTime::parse(s, &Rfc3339).ok()?;
    Some((t.unix_timestamp_nanos() / 1_000) as Micros)
}

pub fn format(us: Micros) -> String {
    let t = OffsetDateTime::from_unix_timestamp_nanos(us as i128 * 1_000).unwrap_or(OffsetDateTime::UNIX_EPOCH);
    t.format(&Rfc3339).unwrap_or_default()
}

pub fn now() -> Micros {
    (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000) as Micros
}

pub const SECOND: Micros = 1_000_000;
pub const MINUTE: Micros = 60 * SECOND;
pub const DAY: Micros = 24 * 60 * MINUTE;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_varying_precision_and_orders_correctly() {
        let a = parse("2024-05-01T10:00:00.9Z").unwrap();
        let b = parse("2024-05-01T10:00:00.123456789Z").unwrap();
        let c = parse("2024-05-01T10:00:01Z").unwrap();
        assert!(b < a && a < c);
        // String order would get this wrong: ".9" > ".123456789" but also "00.9" vs "01".
        assert_eq!(c - a, 100_000);
    }

    #[test]
    fn round_trips() {
        let us = parse("2024-05-01T10:00:00.123456Z").unwrap();
        assert_eq!(parse(&format(us)), Some(us));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse("yesterday"), None);
    }
}
