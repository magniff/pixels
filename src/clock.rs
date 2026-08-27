//! What day it is, which a model has no way of knowing.
//!
//! Its memory of the world stopped when its training did, and it does not know
//! that it stopped, so asked what the date is it either says it cannot tell you
//! or it guesses. Both were observed: asked whether a release dated the day
//! before was recent it said it had come out *today*, and asked what day of the
//! week it was it said Wednesday - which happened to be right, and is the worse
//! of the two answers, because a confident guess that lands teaches you to
//! believe the ones that do not.
//!
//! Local time from the `date` on the machine, the way `curl` fetches and `open`
//! opens: it is there, it knows the timezone, and the alternative is a table of
//! the world's daylight saving rules. Where there is no `date`, the clock is
//! read straight and the answer says it is UTC, because being wrong by an hour
//! without saying so is worse than being right in the wrong zone.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// What the model asked about: today, or some other day.
pub fn about(what: &str) -> Result<String, String> {
    let now = today()?;
    let asked = what.trim().to_lowercase();
    if asked.is_empty() || asked == "today" || asked == "now" {
        // The nudge is in the answer rather than only in the description
        // because this is the moment the mistake gets made. Asked how long
        // until Christmas the model calls this, gets the date, and then counts
        // the days in its head: measured, it answered 86 where the answer was
        // 120. Telling it here - a line before it writes - is worth more than
        // another sentence in a description it read five thousand tokens ago.
        return Ok(format!(
            "{}. To find how far off another day is, ask again for that day \
             rather than counting from this one: you cannot count days and \
             this can.",
            now.said()
        ));
    }
    // A day and a month with no year means the next one there is, which is
    // what somebody asking how long until Christmas means by it.
    let Some(then) = parse(&asked).or_else(|| coming(&asked, &now)) else {
        return Err(format!(
            "{what} is not a date this understands - write it as 2026-12-25, or 12-25 for the \
             next one there is, or ask about today"
        ));
    };
    let gap = days_from_civil(then) - days_from_civil((now.year, now.month, now.day));
    let when = match gap {
        0 => "which is today".to_string(),
        1 => "which is tomorrow".to_string(),
        -1 => "which was yesterday".to_string(),
        d if d > 0 => format!("which is {d} days from today"),
        d => format!("which was {} days ago", -d),
    };
    // A year written from memory is the mistake this tool exists to catch, and
    // it is the one it kept making: asked how long until Christmas it named a
    // year that had already gone. So the answer says where to look next rather
    // than leaving it to work that out.
    let stale = (then.0 < now.year).then(|| {
        format!(
            " If you meant the next one rather than that year's, ask about {}-{:02}-{:02}.",
            if (then.1, then.2) >= (now.month, now.day) {
                now.year
            } else {
                now.year + 1
            },
            then.1,
            then.2
        )
    });
    Ok(format!(
        "{} is a {}, {when}. Today is {}.{}",
        stamp(then),
        weekday(days_from_civil(then)),
        now.said(),
        stale.unwrap_or_default()
    ))
}

/// `12-25` and the like: the next time that day comes round.
fn coming(text: &str, now: &Now) -> Option<(i64, i64, i64)> {
    let mut parts = text.trim().split(['-', '/']);
    let month: i64 = parts.next()?.trim().parse().ok()?;
    let day: i64 = parts.next()?.trim().parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let year = if (month, day) >= (now.month, now.day) {
        now.year
    } else {
        now.year + 1
    };
    Some((year, month, day))
}

/// The day as the machine has it.
struct Now {
    year: i64,
    month: i64,
    day: i64,
    clock: String,
    zone: String,
}

impl Now {
    fn said(&self) -> String {
        format!(
            "{} {}, and the time is {} ({})",
            weekday(days_from_civil((self.year, self.month, self.day))),
            stamp((self.year, self.month, self.day)),
            self.clock,
            self.zone
        )
    }
}

fn today() -> Result<Now, String> {
    // One call, four fields, so the machine's own idea of the timezone is the
    // one that is used.
    if let Ok(out) = Command::new("date").arg("+%Y-%m-%d|%H:%M|%Z").output() {
        if out.status.success() {
            let said = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let mut parts = said.split('|');
            if let (Some(date), Some(clock), Some(zone)) =
                (parts.next(), parts.next(), parts.next())
            {
                if let Some((year, month, day)) = parse(date) {
                    return Ok(Now {
                        year,
                        month,
                        day,
                        clock: clock.to_string(),
                        zone: zone.to_string(),
                    });
                }
            }
        }
    }
    // No `date` to ask, so the clock is read straight and the answer says so.
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "this machine's clock is before 1970".to_string())?
        .as_secs() as i64;
    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
    let rest = secs.rem_euclid(86_400);
    Ok(Now {
        year,
        month,
        day,
        clock: format!("{:02}:{:02}", rest / 3600, (rest % 3600) / 60),
        zone: "UTC".into(),
    })
}

/// `2026-12-25`, and nothing more forgiving: a date written any other way is
/// ambiguous about which number is the month, and guessing at that is how a
/// note ends up with the wrong date in it.
fn parse(text: &str) -> Option<(i64, i64, i64)> {
    let mut parts = text.trim().split('-');
    let year: i64 = parts.next()?.trim().parse().ok()?;
    let month: i64 = parts.next()?.trim().parse().ok()?;
    let day: i64 = parts
        .next()?
        .trim()
        .split(['t', 'T', ' '])
        .next()?
        .parse()
        .ok()?;
    (1..=12).contains(&month).then_some(())?;
    (1..=31).contains(&day).then_some(())?;
    Some((year, month, day))
}

fn stamp((year, month, day): (i64, i64, i64)) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    format!("{day} {} {year}", MONTHS[(month - 1).clamp(0, 11) as usize])
}

fn weekday(days: i64) -> &'static str {
    // 1970-01-01 was a Thursday.
    const DAYS: [&str; 7] = [
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
    ];
    DAYS[days.rem_euclid(7) as usize]
}

/// Days since 1970-01-01, from a calendar date.
///
/// Howard Hinnant's arithmetic: the year is shifted to start in March so that
/// the leap day lands at the end of it and the whole thing is division rather
/// than a table of month lengths. Exact for every date a note will ever hold.
fn days_from_civil((year, month, day): (i64, i64, i64)) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// And back the other way.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    (year + i64::from(month <= 2), month, day)
}
