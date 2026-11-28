use jiff::civil::{Date, Time, Weekday};

use super::schedule::{Schedule, ALL_DAYS};
use super::{RoutineError, ScheduleDto};

pub const NAME_MAX: usize = 60;
pub const TASK_MAX: usize = 20_000;
pub const HOURS_MAX: i64 = 12;

pub fn name(raw: &str) -> Result<String, RoutineError> {
    let name = raw.trim();
    match name.chars().count() {
        0 => Err(RoutineError::new("name-empty", "Enter a name.")),
        count if count > NAME_MAX => Err(RoutineError::new(
            "name-too-long",
            format!("Use at most {NAME_MAX} characters."),
        )),
        _ if name.chars().any(char::is_control) => Err(RoutineError::new(
            "name-invalid",
            "The name cannot contain line breaks or control characters.",
        )),
        _ => Ok(name.to_string()),
    }
}

pub fn task(raw: &str) -> Result<String, RoutineError> {
    if raw.trim().is_empty() {
        return Err(RoutineError::new("task-empty", "Write the task."));
    }
    if raw.chars().count() > TASK_MAX {
        return Err(RoutineError::new(
            "task-too-long",
            format!("Use at most {TASK_MAX} characters."),
        ));
    }
    Ok(raw.to_string())
}

fn digits(part: &str, length: usize) -> Option<i16> {
    if part.len() == length && part.bytes().all(|byte| byte.is_ascii_digit()) {
        part.parse().ok()
    } else {
        None
    }
}

pub fn time(raw: &str) -> Result<Time, RoutineError> {
    let invalid = || RoutineError::new("time-invalid", "Enter a time as HH:MM.");
    let (hour, minute) = raw.split_once(':').ok_or_else(invalid)?;
    let hour = digits(hour, 2).ok_or_else(invalid)?;
    let minute = digits(minute, 2).ok_or_else(invalid)?;
    Time::new(hour as i8, minute as i8, 0, 0).map_err(|_| invalid())
}

pub fn date(raw: &str) -> Result<Date, RoutineError> {
    let invalid = || RoutineError::new("date-invalid", "Enter a date as YYYY-MM-DD.");
    let mut parts = raw.split('-');
    let (Some(year), Some(month), Some(day), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(invalid());
    };
    let year = digits(year, 4).ok_or_else(invalid)?;
    let month = digits(month, 2).ok_or_else(invalid)?;
    let day = digits(day, 2).ok_or_else(invalid)?;
    Date::new(year, month as i8, day as i8).map_err(|_| invalid())
}

fn days(raw: i64) -> Result<u8, RoutineError> {
    if (1..=i64::from(ALL_DAYS)).contains(&raw) {
        Ok(raw as u8)
    } else {
        Err(RoutineError::new("days-empty", "Pick at least one day."))
    }
}

fn every_hours(raw: i64) -> Result<u8, RoutineError> {
    if (1..=HOURS_MAX).contains(&raw) {
        Ok(raw as u8)
    } else {
        Err(RoutineError::new(
            "hours-invalid",
            format!("Pick 1 to {HOURS_MAX} hours."),
        ))
    }
}

fn weekday(raw: i64) -> Result<Weekday, RoutineError> {
    i8::try_from(raw)
        .ok()
        .and_then(|raw| Weekday::from_monday_one_offset(raw).ok())
        .ok_or_else(|| RoutineError::new("weekday-invalid", "Pick a day of the week."))
}

pub fn schedule(dto: &ScheduleDto) -> Result<Schedule, RoutineError> {
    match dto {
        ScheduleDto::Once { date: on, time: at } => Ok(Schedule::Once {
            date: date(on)?,
            at: time(at)?,
        }),
        ScheduleDto::Daily {
            time: at,
            days: mask,
        } => Ok(Schedule::Daily {
            at: time(at)?,
            days: days(*mask)?,
        }),
        ScheduleDto::Hourly {
            time: at,
            every_hours: hours,
            days: mask,
        } => Ok(Schedule::Hourly {
            at: time(at)?,
            every_hours: every_hours(*hours)?,
            days: days(*mask)?,
        }),
        ScheduleDto::Weekly {
            weekday: day,
            time: at,
        } => Ok(Schedule::Weekly {
            weekday: weekday(*day)?,
            at: time(at)?,
        }),
    }
}

pub fn to_dto(schedule: &Schedule) -> ScheduleDto {
    let clock = |at: Time| format!("{:02}:{:02}", at.hour(), at.minute());
    match schedule {
        Schedule::Once { date, at } => ScheduleDto::Once {
            date: format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day()),
            time: clock(*at),
        },
        Schedule::Daily { at, days } => ScheduleDto::Daily {
            time: clock(*at),
            days: i64::from(*days),
        },
        Schedule::Hourly {
            at,
            every_hours,
            days,
        } => ScheduleDto::Hourly {
            time: clock(*at),
            every_hours: i64::from(*every_hours),
            days: i64::from(*days),
        },
        Schedule::Weekly { weekday, at } => ScheduleDto::Weekly {
            weekday: i64::from(weekday.to_monday_one_offset()),
            time: clock(*at),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code<T>(result: Result<T, RoutineError>) -> &'static str {
        result.err().expect("expected an error").code
    }

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(name("  Morning check ").unwrap(), "Morning check");
        assert_eq!(code(name("  ")), "name-empty");
        assert!(name(&"a".repeat(60)).is_ok());
        assert_eq!(code(name(&"a".repeat(61))), "name-too-long");
        assert!(name(&"ก".repeat(60)).is_ok());
        assert_eq!(code(name(&"ก".repeat(61))), "name-too-long");
        assert_eq!(code(name("a\nb")), "name-invalid");
        assert_eq!(code(name("a\u{0}b")), "name-invalid");
    }

    #[test]
    fn tasks_are_not_blank_and_bounded() {
        assert_eq!(code(task(" \n ")), "task-empty");
        assert_eq!(task("  keep spaces\n").unwrap(), "  keep spaces\n");
        assert!(task(&"a".repeat(20_000)).is_ok());
        assert_eq!(code(task(&"a".repeat(20_001))), "task-too-long");
        assert!(task(&"ก".repeat(20_000)).is_ok());
    }

    #[test]
    fn times_need_two_digit_hours_and_minutes() {
        assert_eq!(time("08:05").unwrap().hour(), 8);
        assert!(time("00:00").is_ok());
        assert!(time("23:59").is_ok());
        for bad in [
            "", "8:05", "08:5", "24:00", "12:60", "ab:cd", "08-05", "08:05:00", "+8:05", " 8:05",
        ] {
            assert_eq!(code(time(bad)), "time-invalid", "{bad:?}");
        }
    }

    #[test]
    fn dates_must_be_real_calendar_days() {
        assert_eq!(date("2028-02-29").unwrap().day(), 29);
        for bad in [
            "",
            "2026-02-29",
            "2026-13-01",
            "2026-00-10",
            "26-01-01",
            "2026-1-01",
            "2026/01/01",
            "2026-01-01-01",
            "+026-01-01",
        ] {
            assert_eq!(code(date(bad)), "date-invalid", "{bad:?}");
        }
    }

    #[test]
    fn schedule_fields_are_checked() {
        let daily = |days| ScheduleDto::Daily {
            time: "08:00".into(),
            days,
        };
        assert!(schedule(&daily(127)).is_ok());
        assert!(schedule(&daily(1)).is_ok());
        for bad in [0, 128, -1, 255] {
            assert_eq!(code(schedule(&daily(bad))), "days-empty", "{bad}");
        }
        let hourly = |hours| ScheduleDto::Hourly {
            time: "08:00".into(),
            every_hours: hours,
            days: 127,
        };
        assert!(schedule(&hourly(1)).is_ok());
        assert!(schedule(&hourly(12)).is_ok());
        for bad in [0, 13, -3] {
            assert_eq!(code(schedule(&hourly(bad))), "hours-invalid", "{bad}");
        }
        let weekly = |day| ScheduleDto::Weekly {
            weekday: day,
            time: "08:00".into(),
        };
        assert!(schedule(&weekly(1)).is_ok());
        assert!(schedule(&weekly(7)).is_ok());
        for bad in [0, 8, -1, 4_294_967_297] {
            assert_eq!(code(schedule(&weekly(bad))), "weekday-invalid", "{bad}");
        }
        assert_eq!(
            code(schedule(&ScheduleDto::Once {
                date: "nope".into(),
                time: "08:00".into()
            })),
            "date-invalid"
        );
        assert_eq!(
            code(schedule(&ScheduleDto::Once {
                date: "2026-12-03".into(),
                time: "8".into()
            })),
            "time-invalid"
        );
    }

    #[test]
    fn a_schedule_survives_the_round_trip_through_its_fields() {
        let dtos = [
            ScheduleDto::Once {
                date: "2026-12-03".into(),
                time: "08:00".into(),
            },
            ScheduleDto::Daily {
                time: "08:30".into(),
                days: 31,
            },
            ScheduleDto::Hourly {
                time: "09:05".into(),
                every_hours: 3,
                days: 96,
            },
            ScheduleDto::Weekly {
                weekday: 7,
                time: "23:59".into(),
            },
        ];
        for dto in dtos {
            assert_eq!(to_dto(&schedule(&dto).unwrap()), dto);
        }
    }
}
