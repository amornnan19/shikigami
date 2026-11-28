use jiff::civil::{Date, Time, Weekday};
use jiff::tz::{AmbiguousOffset, TimeZone};
use jiff::Timestamp;

pub const ALL_DAYS: u8 = 127;
pub const WEEKDAYS: u8 = 31;
pub const WEEKENDS: u8 = 96;

const DAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const WEEKDAY_NAMES: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const SEARCH_DAYS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Schedule {
    Once { date: Date, at: Time },
    Daily { at: Time, days: u8 },
    Hourly { at: Time, every_hours: u8, days: u8 },
    Weekly { weekday: Weekday, at: Time },
}

fn day_bit(weekday: Weekday) -> u8 {
    1 << weekday.to_monday_zero_offset()
}

fn resolve(tz: &TimeZone, date: Date, at: Time) -> Option<Timestamp> {
    let ambiguous = tz.to_ambiguous_timestamp(date.to_datetime(at));
    let first = ambiguous.earlier().ok()?;
    match ambiguous.offset() {
        AmbiguousOffset::Gap { .. } => tz
            .following(first)
            .next()
            .map(|transition| transition.timestamp()),
        _ => Some(first),
    }
}

fn runs_on(schedule: &Schedule, date: Date) -> bool {
    match schedule {
        Schedule::Once { date: on, .. } => *on == date,
        Schedule::Daily { days, .. } | Schedule::Hourly { days, .. } => {
            days & day_bit(date.weekday()) != 0
        }
        Schedule::Weekly { weekday, .. } => *weekday == date.weekday(),
    }
}

fn times_of_day(schedule: &Schedule) -> Vec<Time> {
    match schedule {
        Schedule::Once { at, .. } | Schedule::Daily { at, .. } | Schedule::Weekly { at, .. } => {
            vec![*at]
        }
        Schedule::Hourly {
            at, every_hours, ..
        } => {
            let mut times = Vec::new();
            let mut hour = i32::from(at.hour());
            while hour < 24 {
                if let Ok(time) = Time::new(hour as i8, at.minute(), 0, 0) {
                    times.push(time);
                }
                hour += i32::from(*every_hours);
            }
            times
        }
    }
}

pub fn once_instant(schedule: &Schedule, tz: &TimeZone) -> Option<Timestamp> {
    match schedule {
        Schedule::Once { date, at } => resolve(tz, *date, *at),
        _ => None,
    }
}

pub fn next_after(schedule: &Schedule, after: Timestamp, tz: &TimeZone) -> Option<Timestamp> {
    if let Schedule::Once { .. } = schedule {
        return once_instant(schedule, tz).filter(|instant| *instant > after);
    }
    let mut date = tz.to_datetime(after).date().yesterday().ok()?;
    let times = times_of_day(schedule);
    let mut best: Option<Timestamp> = None;
    for _ in 0..SEARCH_DAYS {
        if runs_on(schedule, date) {
            for at in &times {
                if let Some(instant) = resolve(tz, date, *at).filter(|instant| *instant > after) {
                    best = Some(best.map_or(instant, |current| current.min(instant)));
                }
            }
        }
        date = date.tomorrow().ok()?;
    }
    best
}

fn clock(at: Time) -> String {
    format!("{:02}:{:02}", at.hour(), at.minute())
}

fn day_list(days: u8) -> String {
    DAY_NAMES
        .iter()
        .enumerate()
        .filter(|(index, _)| days & (1 << index) != 0)
        .map(|(_, name)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn describe(schedule: &Schedule) -> String {
    match schedule {
        Schedule::Once { date, at } => format!(
            "Once on {} {} {} at {}",
            date.day(),
            MONTH_NAMES[(date.month() - 1) as usize],
            date.year(),
            clock(*at)
        ),
        Schedule::Daily { at, days } => match *days {
            ALL_DAYS => format!("Every day at {}", clock(*at)),
            WEEKDAYS => format!("Every weekday at {}", clock(*at)),
            WEEKENDS => format!("Weekends at {}", clock(*at)),
            _ => format!("Every {} at {}", day_list(*days), clock(*at)),
        },
        Schedule::Hourly {
            at,
            every_hours,
            days,
        } => {
            let interval = if *every_hours == 1 {
                "Every hour".to_string()
            } else {
                format!("Every {every_hours} hours")
            };
            let on = match *days {
                ALL_DAYS => "every day".to_string(),
                WEEKDAYS => "on weekdays".to_string(),
                WEEKENDS => "on weekends".to_string(),
                _ => format!("on {}", day_list(*days)),
            };
            format!("{interval} from {}, {on}", clock(*at))
        }
        Schedule::Weekly { weekday, at } => format!(
            "Every {} at {}",
            WEEKDAY_NAMES[weekday.to_monday_zero_offset() as usize],
            clock(*at)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::{date, time};

    fn tz(name: &str) -> TimeZone {
        TimeZone::get(name).unwrap()
    }

    fn at(tz: &TimeZone, text: &str) -> Timestamp {
        text.parse::<jiff::civil::DateTime>()
            .unwrap()
            .to_zoned_datetime(tz.clone())
            .unwrap()
            .timestamp()
    }

    fn wall(tz: &TimeZone, ts: Option<Timestamp>) -> String {
        ts.map(|ts| {
            ts.to_zoned_datetime(tz.clone())
                .strftime("%Y-%m-%d %H:%M %:z")
                .to_string()
        })
        .unwrap_or_else(|| "none".to_string())
    }

    fn daily(hour: i8, minute: i8, days: u8) -> Schedule {
        Schedule::Daily {
            at: time(hour, minute, 0, 0),
            days,
        }
    }

    #[test]
    fn a_daily_routine_runs_today_when_its_time_is_still_ahead_and_tomorrow_otherwise() {
        let utc = tz("UTC");
        let schedule = daily(8, 0, ALL_DAYS);
        assert_eq!(
            wall(
                &utc,
                next_after(&schedule, at(&utc, "2026-12-01T07:59:00"), &utc)
            ),
            "2026-12-01 08:00 +00:00"
        );
        assert_eq!(
            wall(
                &utc,
                next_after(&schedule, at(&utc, "2026-12-01T08:00:00"), &utc)
            ),
            "2026-12-02 08:00 +00:00"
        );
        assert_eq!(
            wall(
                &utc,
                next_after(&schedule, at(&utc, "2026-12-31T23:59:00"), &utc)
            ),
            "2027-01-01 08:00 +00:00"
        );
    }

    #[test]
    fn the_day_mask_picks_only_the_chosen_days() {
        let utc = tz("UTC");
        let friday_after = at(&utc, "2026-12-04T09:00:00");
        assert_eq!(
            wall(&utc, next_after(&daily(8, 0, WEEKDAYS), friday_after, &utc)),
            "2026-12-07 08:00 +00:00"
        );
        assert_eq!(
            wall(&utc, next_after(&daily(8, 0, WEEKENDS), friday_after, &utc)),
            "2026-12-05 08:00 +00:00"
        );
        let wednesday_only = 0b0000100;
        assert_eq!(
            wall(
                &utc,
                next_after(&daily(8, 0, wednesday_only), friday_after, &utc)
            ),
            "2026-12-09 08:00 +00:00"
        );
        let sunday_only = 0b1000000;
        assert_eq!(
            wall(
                &utc,
                next_after(&daily(8, 0, sunday_only), friday_after, &utc)
            ),
            "2026-12-06 08:00 +00:00"
        );
        let monday_only = 0b0000001;
        assert_eq!(
            wall(
                &utc,
                next_after(&daily(8, 0, monday_only), friday_after, &utc)
            ),
            "2026-12-07 08:00 +00:00"
        );
    }

    #[test]
    fn a_weekly_routine_runs_on_its_weekday() {
        let utc = tz("UTC");
        let schedule = Schedule::Weekly {
            weekday: Weekday::Monday,
            at: time(8, 0, 0, 0),
        };
        assert_eq!(
            wall(
                &utc,
                next_after(&schedule, at(&utc, "2026-12-07T07:00:00"), &utc)
            ),
            "2026-12-07 08:00 +00:00"
        );
        assert_eq!(
            wall(
                &utc,
                next_after(&schedule, at(&utc, "2026-12-07T08:00:00"), &utc)
            ),
            "2026-12-14 08:00 +00:00"
        );
        assert_eq!(
            wall(
                &utc,
                next_after(&schedule, at(&utc, "2026-12-08T00:00:00"), &utc)
            ),
            "2026-12-14 08:00 +00:00"
        );
    }

    #[test]
    fn an_hourly_routine_stays_inside_its_day_and_starts_over_the_next_chosen_day() {
        let utc = tz("UTC");
        let schedule = Schedule::Hourly {
            at: time(9, 30, 0, 0),
            every_hours: 5,
            days: ALL_DAYS,
        };
        let expected = ["09:30", "14:30", "19:30"];
        let mut cursor = at(&utc, "2026-12-01T00:00:00");
        for slot in expected {
            cursor = next_after(&schedule, cursor, &utc).unwrap();
            assert_eq!(
                wall(&utc, Some(cursor)),
                format!("2026-12-01 {slot} +00:00")
            );
        }
        assert_eq!(
            wall(&utc, next_after(&schedule, cursor, &utc)),
            "2026-12-02 09:30 +00:00"
        );
        let weekdays = Schedule::Hourly {
            at: time(22, 0, 0, 0),
            every_hours: 1,
            days: WEEKDAYS,
        };
        assert_eq!(
            wall(
                &utc,
                next_after(&weekdays, at(&utc, "2026-12-04T22:00:00"), &utc)
            ),
            "2026-12-04 23:00 +00:00"
        );
        assert_eq!(
            wall(
                &utc,
                next_after(&weekdays, at(&utc, "2026-12-04T23:00:00"), &utc)
            ),
            "2026-12-07 22:00 +00:00"
        );
    }

    #[test]
    fn a_once_routine_runs_one_time_and_never_in_the_past() {
        let utc = tz("UTC");
        let schedule = Schedule::Once {
            date: date(2026, 12, 3),
            at: time(8, 0, 0, 0),
        };
        assert_eq!(
            wall(
                &utc,
                next_after(&schedule, at(&utc, "2026-12-03T07:00:00"), &utc)
            ),
            "2026-12-03 08:00 +00:00"
        );
        assert_eq!(
            next_after(&schedule, at(&utc, "2026-12-03T08:00:00"), &utc),
            None
        );
        assert_eq!(
            wall(&utc, once_instant(&schedule, &utc)),
            "2026-12-03 08:00 +00:00"
        );
    }

    #[test]
    fn the_zone_decides_the_clock_time() {
        let tokyo = tz("Asia/Tokyo");
        let schedule = daily(8, 0, ALL_DAYS);
        let next = next_after(&schedule, at(&tz("UTC"), "2026-11-30T12:00:00"), &tokyo);
        assert_eq!(wall(&tokyo, next), "2026-12-01 08:00 +09:00");
        assert_eq!(next.unwrap().to_string(), "2026-11-30T23:00:00Z");
    }

    #[test]
    fn a_spring_forward_gap_runs_at_the_first_valid_minute() {
        let new_york = tz("America/New_York");
        let before = at(&new_york, "2026-03-07T12:00:00");
        for (hour, minute) in [(2, 0), (2, 30), (2, 59)] {
            let next = next_after(&daily(hour, minute, ALL_DAYS), before, &new_york);
            assert_eq!(
                wall(&new_york, next),
                "2026-03-08 03:00 -04:00",
                "{hour}:{minute}"
            );
        }
    }

    #[test]
    fn a_berlin_gap_runs_at_the_first_valid_minute_and_the_next_day_is_normal() {
        let berlin = tz("Europe/Berlin");
        let schedule = daily(2, 30, ALL_DAYS);
        let first = next_after(&schedule, at(&berlin, "2026-03-28T12:00:00"), &berlin).unwrap();
        assert_eq!(wall(&berlin, Some(first)), "2026-03-29 03:00 +02:00");
        assert_eq!(
            wall(&berlin, next_after(&schedule, first, &berlin)),
            "2026-03-30 02:30 +02:00"
        );
    }

    #[test]
    fn an_hourly_run_that_lands_in_a_gap_does_not_run_twice() {
        let new_york = tz("America/New_York");
        let schedule = Schedule::Hourly {
            at: time(0, 0, 0, 0),
            every_hours: 1,
            days: ALL_DAYS,
        };
        let mut cursor = at(&new_york, "2026-03-08T00:00:00");
        let mut seen = Vec::new();
        for _ in 0..4 {
            cursor = next_after(&schedule, cursor, &new_york).unwrap();
            seen.push(wall(&new_york, Some(cursor)));
        }
        assert_eq!(
            seen,
            [
                "2026-03-08 01:00 -05:00",
                "2026-03-08 03:00 -04:00",
                "2026-03-08 04:00 -04:00",
                "2026-03-08 05:00 -04:00",
            ]
        );
    }

    #[test]
    fn a_repeated_wall_time_runs_at_its_first_occurrence_only() {
        let new_york = tz("America/New_York");
        let schedule = daily(1, 30, ALL_DAYS);
        let first = next_after(&schedule, at(&new_york, "2026-11-02T12:00:00"), &new_york).unwrap();
        assert_eq!(wall(&new_york, Some(first)), "2026-11-03 01:30 -05:00");
        let before =
            next_after(&schedule, at(&new_york, "2026-10-31T12:00:00"), &new_york).unwrap();
        assert_eq!(wall(&new_york, Some(before)), "2026-11-01 01:30 -04:00");
        let after = next_after(&schedule, before, &new_york).unwrap();
        assert_eq!(wall(&new_york, Some(after)), "2026-11-02 01:30 -05:00");
    }

    #[test]
    fn a_berlin_fall_back_repeats_nothing() {
        let berlin = tz("Europe/Berlin");
        let schedule = daily(2, 30, ALL_DAYS);
        let first = next_after(&schedule, at(&berlin, "2026-10-24T12:00:00"), &berlin).unwrap();
        assert_eq!(wall(&berlin, Some(first)), "2026-10-25 02:30 +02:00");
        let second = next_after(&schedule, first, &berlin).unwrap();
        assert_eq!(wall(&berlin, Some(second)), "2026-10-26 02:30 +01:00");
    }

    #[test]
    fn descriptions_read_in_plain_words() {
        let eight = time(8, 0, 0, 0);
        let cases = [
            (
                Schedule::Once {
                    date: date(2026, 12, 3),
                    at: eight,
                },
                "Once on 3 Dec 2026 at 08:00",
            ),
            (daily(8, 0, ALL_DAYS), "Every day at 08:00"),
            (daily(8, 0, WEEKDAYS), "Every weekday at 08:00"),
            (daily(8, 0, WEEKENDS), "Weekends at 08:00"),
            (daily(8, 0, 0b0010101), "Every Mon, Wed, Fri at 08:00"),
            (
                Schedule::Hourly {
                    at: time(9, 0, 0, 0),
                    every_hours: 3,
                    days: ALL_DAYS,
                },
                "Every 3 hours from 09:00, every day",
            ),
            (
                Schedule::Hourly {
                    at: time(9, 0, 0, 0),
                    every_hours: 1,
                    days: WEEKDAYS,
                },
                "Every hour from 09:00, on weekdays",
            ),
            (
                Schedule::Hourly {
                    at: time(9, 0, 0, 0),
                    every_hours: 2,
                    days: WEEKENDS,
                },
                "Every 2 hours from 09:00, on weekends",
            ),
            (
                Schedule::Hourly {
                    at: time(9, 5, 0, 0),
                    every_hours: 4,
                    days: 0b0000101,
                },
                "Every 4 hours from 09:05, on Mon, Wed",
            ),
            (
                Schedule::Weekly {
                    weekday: Weekday::Monday,
                    at: eight,
                },
                "Every Monday at 08:00",
            ),
        ];
        for (schedule, text) in cases {
            assert_eq!(describe(&schedule), text);
        }
    }
}
