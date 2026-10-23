#![cfg_attr(not(windows), allow(dead_code))]
//! Text for the virtual list's columns.
//!
//! The list view still asks for this text through `LVN_GETDISPINFO`. Size and
//! Date modified are also painted with DirectWrite. A directory has no size
//! string. Date modified is UTC `YYYY-MM-DD HH:MM` so tests do not depend on
//! the machine's zone. This module does not read a directory.

use std::time::SystemTime;

use fx_core::FileEntry;

pub const NAME: i32 = 0;
pub const SIZE: i32 = 1;
pub const MODIFIED: i32 = 2;
pub const COUNT: i32 = 3;
/// Width of the Size column. Not a performance budget.
pub const SIZE_WIDTH: i32 = 96;
/// Width of the Date modified column. Not a performance budget.
pub const MODIFIED_WIDTH: i32 = 140;

const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

pub fn title(subitem: i32) -> &'static str {
    match subitem {
        NAME => "Name",
        SIZE => "Size",
        MODIFIED => "Date modified",
        _ => "",
    }
}

pub fn text(subitem: i32, entry: &FileEntry) -> String {
    match subitem {
        NAME => entry.name.clone(),
        SIZE => format_size(entry.is_directory, entry.size),
        MODIFIED => format_modified(entry.last_write),
        _ => String::new(),
    }
}

/// Blank for a directory. Otherwise a 1024-based size with one decimal place.
pub fn format_size(is_directory: bool, size: u64) -> String {
    if is_directory {
        return String::new();
    }
    if size < 1024 {
        return format!("{size} B");
    }
    let mut whole = size;
    let mut remainder = 0u64;
    let mut unit = 0usize;
    while whole >= 1024 && unit < UNITS.len() - 1 {
        remainder = whole % 1024;
        whole /= 1024;
        unit += 1;
    }
    let mut tenth = (remainder * 10 + 512) / 1024;
    if tenth >= 10 {
        whole += 1;
        tenth = 0;
        if whole >= 1024 && unit < UNITS.len() - 1 {
            whole /= 1024;
            unit += 1;
        }
    }
    if tenth == 0 {
        format!("{whole} {}", UNITS[unit])
    } else {
        format!("{whole}.{tenth} {}", UNITS[unit])
    }
}

/// UTC minute precision. Empty when `time` is before the Unix epoch.
pub fn format_modified(time: SystemTime) -> String {
    let Ok(elapsed) = time.duration_since(SystemTime::UNIX_EPOCH) else {
        return String::new();
    };
    let secs = elapsed.as_secs();
    let days = i64::try_from(secs / 86_400).unwrap_or(i64::MAX);
    let day_secs = (secs % 86_400) as u32;
    let hour = day_secs / 3_600;
    let minute = (day_secs % 3_600) / 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

/// Howard Hinnant's `civil_from_days`. `days` is the Unix day number.
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = if month_part < 10 {
        month_part + 3
    } else {
        month_part - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (year as i32, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn entry(name: &str, size: u64, directory: bool, modified: SystemTime) -> FileEntry {
        FileEntry {
            name: name.to_string(),
            attributes: 0,
            size,
            created: modified,
            last_write: modified,
            is_directory: directory,
            is_reparse_point: false,
        }
    }

    #[test]
    fn sizes_use_1024_and_one_decimal() {
        assert_eq!(format_size(true, 4096), "");
        assert_eq!(format_size(false, 0), "0 B");
        assert_eq!(format_size(false, 1023), "1023 B");
        assert_eq!(format_size(false, 1024), "1 KB");
        assert_eq!(format_size(false, 1536), "1.5 KB");
        assert_eq!(format_size(false, 1024 * 1024), "1 MB");
        assert_eq!(format_size(false, 1536 * 1024), "1.5 MB");
        assert_eq!(format_size(false, 1024 * 1024 - 1), "1 MB");
    }

    #[test]
    fn modified_is_utc_to_the_minute() {
        assert_eq!(format_modified(SystemTime::UNIX_EPOCH), "1970-01-01 00:00");
        let new_year = SystemTime::UNIX_EPOCH + Duration::from_secs(1_609_459_200);
        assert_eq!(format_modified(new_year), "2021-01-01 00:00");
        let leap = SystemTime::UNIX_EPOCH + Duration::from_secs(1_582_981_500);
        assert_eq!(format_modified(leap), "2020-02-29 13:05");
        let before = SystemTime::UNIX_EPOCH - Duration::from_secs(1);
        assert_eq!(format_modified(before), "");
    }

    #[test]
    fn column_text_follows_the_subitem() {
        let when = SystemTime::UNIX_EPOCH + Duration::from_secs(1_609_459_200);
        let file = entry("notes.txt", 1536, false, when);
        assert_eq!(text(NAME, &file), "notes.txt");
        assert_eq!(text(SIZE, &file), "1.5 KB");
        assert_eq!(text(MODIFIED, &file), "2021-01-01 00:00");
        assert_eq!(title(NAME), "Name");
        assert_eq!(title(SIZE), "Size");
        assert_eq!(title(MODIFIED), "Date modified");
        let folder = entry("docs", 99, true, when);
        assert_eq!(text(SIZE, &folder), "");
    }
}
