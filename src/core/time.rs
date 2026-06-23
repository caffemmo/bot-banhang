use chrono::{DateTime, FixedOffset, NaiveDateTime, Utc};

pub fn format_vietnam_time(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return "-".to_string();
    }

    parse_datetime_utc(raw)
        .map(format_vietnam_datetime)
        .unwrap_or_else(|| raw.to_string())
}

pub fn format_vietnam_datetime(dt: DateTime<Utc>) -> String {
    let vn_offset = FixedOffset::east_opt(7 * 3600).expect("valid Vietnam UTC offset");
    dt.with_timezone(&vn_offset)
        .format("%d/%m/%Y %H:%M:%S")
        .to_string()
}

pub fn format_optional_vietnam_time(raw: Option<&str>) -> String {
    raw.map(format_vietnam_time)
        .filter(|value| value != "-")
        .unwrap_or_else(|| "-".to_string())
}

fn parse_datetime_utc(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")
                .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
                .ok()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_rfc3339_as_vietnam_time() {
        assert_eq!(
            format_vietnam_time("2026-06-23T12:36:25.768596755+00:00"),
            "23/06/2026 19:36:25"
        );
    }

    #[test]
    fn formats_sqlite_datetime_as_vietnam_time() {
        assert_eq!(
            format_vietnam_time("2026-06-23 12:36:25"),
            "23/06/2026 19:36:25"
        );
    }
}
