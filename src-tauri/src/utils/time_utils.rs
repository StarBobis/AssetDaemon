/*
 * Time Utilities -- Provides date formatting, leap year checking, and other general time processing capabilities.
 *
 * Follows Soul.md conventions: organized as struct + impl, no free functions.
 */

/**
 * Time processing utility class.
 *
 * Contains static methods for SystemTime formatting, Unix days to date conversion, leap year checking, etc.
 * Does not involve any business logic; it is a pure utility method.
 */
pub struct TimeUtils;

impl TimeUtils {
    /**
     * Formats SystemTime as a "YYYY-MM-DD HH:MM:SS" string.
     */
    pub fn format_time(time: std::time::SystemTime) -> String {
        let duration = time
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let total_secs = duration.as_secs();
        let days = total_secs / 86400;
        let time_secs = total_secs % 86400;
        let hours = time_secs / 3600;
        let minutes = (time_secs % 3600) / 60;
        let seconds = time_secs % 60;
        let (year, month, day) = Self::days_to_date(days as i64);
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            year, month, day, hours, minutes, seconds
        )
    }

    /** Converts Unix days to a date (year/month/day) */
    fn days_to_date(mut days: i64) -> (i64, i64, i64) {
        let mut year = 1970i64;
        loop {
            let days_in_year = if Self::is_leap_year(year) { 366 } else { 365 };
            if days < days_in_year {
                break;
            }
            days -= days_in_year;
            year += 1;
        }
        let month_days = if Self::is_leap_year(year) {
            [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        } else {
            [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        };
        let mut month = 1i64;
        for &md in month_days.iter() {
            if days < md {
                break;
            }
            days -= md;
            month += 1;
        }
        (year, month, days + 1)
    }

    /** Checks whether a year is a leap year */
    fn is_leap_year(year: i64) -> bool {
        (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
    }

    /**
     * Formats seconds into a human-readable duration.
     *
     * < 1 minute -> "Xs"
     * >= 1 minute -> "XmYs"
     * >= 1 hour -> "XhYmZs"
     */
    pub fn format_duration(secs_f64: f64) -> String {
        let total_secs = secs_f64 as u64;
        let hours = total_secs / 3600;
        let minutes = (total_secs % 3600) / 60;
        let seconds = total_secs % 60;
        if hours > 0 {
            format!("{}h{}m{}s", hours, minutes, seconds)
        } else if minutes > 0 {
            format!("{}m{}s", minutes, seconds)
        } else {
            format!("{:.1}s", secs_f64)
        }
    }
}
