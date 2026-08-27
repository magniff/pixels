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
            "{}. Do not work out how far off another day is by counting from \
             this one - you cannot count days and this can. Ask it again with \
             the day instead: 12-25 for Christmas, 11-03 for the third of \
             November. It answers with the next one and the one before, so \
             that is both directions in a single question.",
            now.said()
        ));
    }
    // A day and a month with no year is not one day, it is one that comes
    // round - and asked about one of those, people mean both ends of it. "When
    // is the next Christmas and how long since the last" is one question, and
    // answering half of it is how the other half gets counted by hand: asked
    // that, the model called this once and then worked the rest out itself,
    // making the last Christmas 274 days ago, then 306, then a Wednesday. It
    // was 245 days and a Thursday. So a recurring day answers about both, and
    // there is nothing left to count.
    // A month by name, which `parse` will not take and which nobody writing to
    // a person would think twice about.
    let asked = match named(&asked) {
        Some((Some(year), month, day)) => format!("{year}-{month:02}-{day:02}"),
        Some((None, month, day)) => format!("{month:02}-{day:02}"),
        None => asked,
    };
    if parse(&asked).is_none() {
        if let Some(next) = coming(&asked, &now) {
            let now_ymd = (now.year, now.month, now.day);
            let here = days_from_civil(now_ymd);
            let last = (next.0 - 1, next.1, next.2);
            let (to, since) = (days_from_civil(next) - here, here - days_from_civil(last));
            let long = |a, b| match spanned(a, b) {
                s if s.is_empty() => String::new(),
                s => format!(" - {s}"),
            };
            let ahead = format!("{}{}", plural(to, "from today"), long(now_ymd, next));
            let behind = format!("{} ago{}", plural(since, ""), long(last, now_ymd));
            return Ok(format!(
                "The next {} is {}, a {}, {}. The one before was {}, a {}, {}. Today is {}.",
                stamp((0, next.1, next.2)).rsplit_once(' ').map_or_else(
                    || stamp((0, next.1, next.2)),
                    |(day_month, _)| day_month.to_string()
                ),
                stamp(next),
                weekday(days_from_civil(next)),
                ahead,
                stamp(last),
                weekday(days_from_civil(last)),
                behind,
                now.said()
            ));
        }
    }

    let Some(then) = parse(&asked).or_else(|| coming(&asked, &now)) else {
        return Err(format!(
            "{what} is not a date this understands - write it as 2026-12-25, or 12-25 for the \
             next one there is, or ask about today"
        ));
    };
    let gap = days_from_civil(then) - days_from_civil((now.year, now.month, now.day));
    let now_ymd = (now.year, now.month, now.day);
    // In years and months as well, past a point. See `spanned`.
    let long = |a, b| match spanned(a, b) {
        s if s.is_empty() => String::new(),
        s => format!(" - {s}"),
    };
    let when = match gap {
        0 => "which is today".to_string(),
        1 => "which is tomorrow".to_string(),
        -1 => "which was yesterday".to_string(),
        d if d > 0 => format!("which is {d} days from today{}", long(now_ymd, then)),
        d => format!("which was {} days ago{}", -d, long(then, now_ymd)),
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
    // The same day a year either side, because a date asked about by name is
    // usually one that comes round, and the question after "when is the next"
    // is "and when was the last". Asked with the year spelled out, the model
    // got the one it asked for right and then invented the other: 366 days,
    // 80 days, where it was 245.
    let before = (then.0 - 1, then.1, then.2);
    let gap_before = days_from_civil((now.year, now.month, now.day)) - days_from_civil(before);
    let neighbour = format!(
        " The same day a year earlier, {}, was a {} and was {} days ago.",
        stamp(before),
        weekday(days_from_civil(before)),
        gap_before
    );
    Ok(format!(
        "{} is a {}, {when}.{neighbour} Today is {}.{}",
        stamp(then),
        weekday(days_from_civil(then)),
        now.said(),
        stale.unwrap_or_default()
    ))
}

/// How long between two days, in the units people use for it.
///
/// Days are exact and useless past a certain size: told somebody was 612 days
/// old, the model divided by 365, rounded, and announced that a child born in
/// December 2024 had turned two. She is one. Whole calendar years and months
/// are what anybody means by an age, and counting them is not something to
/// leave to a model with a division sign.
fn spanned(from: (i64, i64, i64), to: (i64, i64, i64)) -> String {
    let (mut years, mut months) = (to.0 - from.0, to.1 - from.1);
    if to.2 < from.2 {
        months -= 1;
    }
    if months < 0 {
        years -= 1;
        months += 12;
    }
    let say = |n: i64, unit: &str| format!("{n} {unit}{}", if n == 1 { "" } else { "s" });
    match (years, months) {
        (0, 0) => String::new(),
        (0, m) => say(m, "month"),
        (y, 0) => say(y, "year"),
        (y, m) => format!("{} and {}", say(y, "year"), say(m, "month")),
    }
}

/// A count of days, said the way somebody would say it.
fn plural(days: i64, tail: &str) -> String {
    let n = match days {
        0 => "today".to_string(),
        1 => "1 day".to_string(),
        d => format!("{d} days"),
    };
    match (days, tail.is_empty()) {
        (0, _) => n,
        (_, true) => n,
        _ => format!("{n} {tail}"),
    }
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

/// The months, by the first three letters of each, which is all anybody
/// abbreviates them to.
const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// A date written with the month's name in it, in whichever order.
///
/// `31 jul 1989`, `July 31, 1989`, `jul 31 1989`. The rule below refuses
/// everything but `1989-07-31` because `07/31` and `31/07` are the same six
/// characters meaning two different days - but a month written out is not
/// ambiguous, and refusing it cost the answer: asked how many days somebody
/// had been alive from "jul 31 1989", the model could not turn that into a
/// date this would take, tried to do the arithmetic inside the calculator
/// instead, and was out by eight hundred days.
///
/// The year may be missing, in which case this is a day that comes round and
/// the caller treats it as one.
fn named(text: &str) -> Option<(Option<i64>, i64, i64)> {
    let words: Vec<&str> = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let month = words.iter().find_map(|w| {
        let w = w.to_lowercase();
        (w.len() >= 3).then(|| {
            MONTHS
                .iter()
                .position(|m| m.starts_with(&w) || w.starts_with(&m[..3]))
        })?
    })? as i64
        + 1;
    let mut year = None;
    let mut day = None;
    for w in &words {
        let Ok(n) = w.parse::<i64>() else { continue };
        if w.len() == 4 && (1000..=9999).contains(&n) {
            year.get_or_insert(n);
        } else if (1..=31).contains(&n) {
            day.get_or_insert(n);
        }
    }
    Some((year, month, day?))
}

/// `2026-12-25`, and nothing more forgiving *in figures*: a date written any
/// other way in numbers alone is ambiguous about which of them is the month,
/// and guessing at that is how a note ends up with the wrong date in it. A
/// month with a name is not in doubt, and [`named`] takes those.
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
    let name = MONTHS[(month - 1).clamp(0, 11) as usize];
    let mut titled = name.to_string();
    titled[..1].make_ascii_uppercase();
    format!("{day} {titled} {year}")
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
