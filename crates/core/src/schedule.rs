//! Wall-clock schedule math for `AtTime` timers.

use chrono::{DateTime, Datelike, Days, NaiveTime, TimeZone};

use crate::model::Repeat;

/// Next instant strictly after `now` at which an `AtTime` timer should fire.
///
/// Days whose local time does not exist (DST gap) are skipped. Returns `None`
/// only if no matching day exists within the next 8 days (e.g. empty `Days`).
pub fn next_occurrence<Tz: TimeZone>(time: NaiveTime, repeat: &Repeat, now: &DateTime<Tz>) -> Option<DateTime<Tz>> {
    let tz = now.timezone();
    let today = now.date_naive();
    for offset in 0..=7 {
        let day = today.checked_add_days(Days::new(offset))?;
        if !repeat.includes(day.weekday()) {
            continue;
        }
        let Some(at) = tz.from_local_datetime(&day.and_time(time)).earliest() else {
            continue;
        };
        if at > *now {
            return Some(at);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, Weekday};

    fn tz() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<FixedOffset> {
        tz().with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    fn hm(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn later_today() {
        // 2026-11-09 is a Monday
        let now = at(2026, 11, 9, 8, 0);
        assert_eq!(next_occurrence(hm(9, 45), &Repeat::Daily, &now), Some(at(2026, 11, 9, 9, 45)));
    }

    #[test]
    fn already_passed_rolls_to_tomorrow() {
        let now = at(2026, 11, 9, 10, 0);
        assert_eq!(next_occurrence(hm(9, 45), &Repeat::Once, &now), Some(at(2026, 11, 10, 9, 45)));
    }

    #[test]
    fn exactly_now_is_not_next() {
        let now = at(2026, 11, 9, 9, 45);
        assert_eq!(next_occurrence(hm(9, 45), &Repeat::Daily, &now), Some(at(2026, 11, 10, 9, 45)));
    }

    #[test]
    fn weekdays_skip_weekend() {
        // Friday after the time -> Monday
        let now = at(2026, 11, 13, 18, 0);
        assert_eq!(next_occurrence(hm(9, 45), &Repeat::Weekdays, &now), Some(at(2026, 11, 16, 9, 45)));
    }

    #[test]
    fn specific_days_wrap_a_full_week() {
        // Monday after the time, only Mondays -> next Monday
        let now = at(2026, 11, 9, 10, 0);
        let repeat = Repeat::Days { days: vec![Weekday::Mon] };
        assert_eq!(next_occurrence(hm(9, 45), &repeat, &now), Some(at(2026, 11, 16, 9, 45)));
    }

    #[test]
    fn empty_days_never_fire() {
        let now = at(2026, 11, 9, 10, 0);
        assert_eq!(next_occurrence(hm(9, 45), &Repeat::Days { days: vec![] }, &now), None);
    }
}
