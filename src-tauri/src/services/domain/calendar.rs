//! The calendar trigger source: fire at set times on set days, and say who
//! is responsible on each of them.
//!
//! ```yaml
//! source:
//!   type: calendar
//!   config:
//!     times: ["09:00"]                 # local time of day, one or more
//!     days: [mon, tue, wed, thu, fri]  # empty: every day
//!     skipDates: ["2026-12-25"]        # holidays: no fire
//!     people: [alice, bob]             # take turns...
//!     rotate: week                     # ...a week each (or `day`)
//!     startDate: "2026-10-26"          # alice's first day
//!     byDay: {fri: carol}              # fixed weekday owners beat turns
//!     cover:                           # stand-ins beat everything
//!       - {person: dana, from: "2026-11-02", to: "2026-11-06", backup: bob}
//!     backup: erin                     # default: whose turn is next
//!     backupAfterMinutes: 60           # also hand over if not done by then
//! ```
//!
//! Who is responsible on a day: the last `cover` entry that spans it, else
//! its `byDay` owner, else whose turn it is. When the calendar names people
//! but nobody is responsible on a day (say `byDay` covers only Mondays), the
//! trigger doesn't fire that day. Without people it simply fires on time.
//!
//! **Backup.** Each day also has a backup: the cover's `backup`, else the
//! calendar's `backup`, else the next person in the rotation (never the
//! primary). The backup takes over when the primary's run fails, isn't
//! finished `backupAfterMinutes` after it started, or never starts (see
//! `services::triggers::calendar`).
//!
//! People are named as the relay shows them (an ID or a name); the agent
//! finds and reaches them with its own tools. This is a schedule, not a
//! directory of people.
//!
//! Everything here is local wall-clock time without a zone; the scheduler
//! maps it onto the machine's time zone.

use std::collections::BTreeMap;

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use serde::{Deserialize, Serialize};

use super::{DomainError, DomainResult};

const MAX_PERSON_CHARS: usize = 100;
const MAX_TIMES: usize = 24;
/// How far ahead the next fire is looked for
const LOOKAHEAD_DAYS: i64 = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Day {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Day {
    fn of(date: NaiveDate) -> Self {
        match date.weekday() {
            Weekday::Mon => Self::Mon,
            Weekday::Tue => Self::Tue,
            Weekday::Wed => Self::Wed,
            Weekday::Thu => Self::Thu,
            Weekday::Fri => Self::Fri,
            Weekday::Sat => Self::Sat,
            Weekday::Sun => Self::Sun,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Rotate {
    /// The next person each scheduled day (skipped days don't use a turn)
    Day,
    /// Seven days each, counted from `startDate`
    #[default]
    Week,
}

/// Someone standing in from `from` to `to` (both included).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cover {
    pub person: String,
    pub from: NaiveDate,
    pub to: NaiveDate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CalendarSpec {
    #[serde(with = "hhmm")]
    times: Vec<NaiveTime>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    days: Vec<Day>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    skip_dates: Vec<NaiveDate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    people: Vec<String>,
    #[serde(default)]
    rotate: Rotate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    start_date: Option<NaiveDate>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    by_day: BTreeMap<Day, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    cover: Vec<Cover>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    backup: Option<String>,
    /// Hand over to the backup when the primary's run isn't finished by
    /// then. Without it, the backup steps in only on a failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    backup_after_minutes: Option<u32>,
}

/// Why someone is responsible on a day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DutyReason {
    Rotation,
    ByDay,
    Cover,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Duty {
    pub person: String,
    pub reason: DutyReason,
    /// With a cover: who it would have been
    pub covering_for: Option<String>,
    /// Who takes over if the primary's run fails (never `person`)
    pub backup: Option<String>,
}

/// One scheduled fire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarFire {
    pub at: NaiveDateTime,
    /// `None` when the calendar names no people
    pub duty: Option<Duty>,
}

impl CalendarSpec {
    /// Parses and checks a `source.config` mapping.
    pub fn from_value(config: serde_json::Value) -> DomainResult<Self> {
        let mut spec: Self =
            serde_json::from_value(config).map_err(|e| DomainError::parse("calendar config", e))?;
        spec.times.sort();
        spec.times.dedup();
        spec.check()?;
        Ok(spec)
    }

    fn check(&self) -> DomainResult<()> {
        let invalid = |field: &'static str, reason: String| -> DomainResult<()> {
            Err(DomainError::invalid("calendar trigger", field, reason))
        };
        if self.times.is_empty() {
            return invalid("times", "needs at least one time, like 09:00".into());
        }
        if self.times.len() > MAX_TIMES {
            return invalid("times", format!("has more than {MAX_TIMES} times"));
        }
        let names = self
            .people
            .iter()
            .chain(self.by_day.values())
            .chain(self.cover.iter().map(|c| &c.person))
            .chain(self.cover.iter().filter_map(|c| c.backup.as_ref()))
            .chain(self.backup.as_ref());
        for name in names {
            if name.trim().is_empty() || name.chars().count() > MAX_PERSON_CHARS {
                return invalid(
                    "people",
                    format!("'{name}' must be 1 to {MAX_PERSON_CHARS} characters"),
                );
            }
        }
        if !self.people.is_empty() && self.start_date.is_none() {
            return invalid(
                "startDate",
                "is required when people take turns (the first person's first day)".into(),
            );
        }
        if let Some(cover) = self.cover.iter().find(|c| c.to < c.from) {
            return invalid(
                "cover",
                format!("{}'s cover ends before it starts", cover.person),
            );
        }
        if self.backup_after_minutes == Some(0) {
            return invalid("backupAfterMinutes", "must be at least 1".into());
        }
        Ok(())
    }

    pub fn backup_after_minutes(&self) -> Option<u32> {
        self.backup_after_minutes
    }

    fn names_people(&self) -> bool {
        !self.people.is_empty() || !self.by_day.is_empty() || !self.cover.is_empty()
    }

    /// Whether any of `names` (one person's names) appears in the calendar.
    pub fn mentions(&self, names: &[String]) -> bool {
        self.people
            .iter()
            .chain(self.by_day.values())
            .chain(self.cover.iter().map(|c| &c.person))
            .chain(self.cover.iter().filter_map(|c| c.backup.as_ref()))
            .chain(self.backup.as_ref())
            .any(|person| is_one_of(person, names))
    }

    /// Whether `date` is on the schedule at all (ignoring who's on duty).
    fn scheduled(&self, date: NaiveDate) -> bool {
        (self.days.is_empty() || self.days.contains(&Day::of(date)))
            && !self.skip_dates.contains(&date)
    }

    /// The index of whose turn it is (unbounded; take it modulo `people`).
    fn turn_index(&self, date: NaiveDate) -> Option<usize> {
        let start = self.start_date?;
        if self.people.is_empty() || date < start {
            return None;
        }
        let index = match self.rotate {
            Rotate::Week => (date - start).num_days() / 7,
            Rotate::Day => start
                .iter_days()
                .take_while(|d| *d < date)
                .filter(|d| self.scheduled(*d))
                .count() as i64,
        };
        Some(index as usize)
    }

    fn turn(&self, date: NaiveDate) -> Option<&String> {
        self.turn_index(date)
            .map(|index| &self.people[index % self.people.len()])
    }

    /// Whose turn is next after `date`'s, skipping `primary`.
    fn next_turn(&self, date: NaiveDate, primary: &str) -> Option<&String> {
        let index = self.turn_index(date)?;
        (1..self.people.len())
            .map(|step| &self.people[(index + step) % self.people.len()])
            .find(|person| *person != primary)
    }

    /// Who is responsible on `date`, whether or not it's scheduled.
    pub fn duty_on(&self, date: NaiveDate) -> Option<Duty> {
        let regular = self
            .by_day
            .get(&Day::of(date))
            .map(|p| (p, DutyReason::ByDay))
            .or_else(|| self.turn(date).map(|p| (p, DutyReason::Rotation)));
        let cover = self
            .cover
            .iter()
            .rev()
            .find(|c| (c.from..=c.to).contains(&date));
        let (person, reason, covering_for, explicit_backup) = match cover {
            Some(cover) => (
                &cover.person,
                DutyReason::Cover,
                regular
                    .map(|(p, _)| p.clone())
                    .filter(|p| *p != cover.person),
                cover.backup.as_ref().or(self.backup.as_ref()),
            ),
            None => {
                let (person, reason) = regular?;
                (person, reason, None, self.backup.as_ref())
            }
        };
        let backup = explicit_backup
            .filter(|b| *b != person)
            .or_else(|| self.next_turn(date, person))
            .cloned();
        Some(Duty {
            person: person.clone(),
            reason,
            covering_for,
            backup,
        })
    }

    /// The first fire strictly after `after`, if one falls in the next
    /// year or so.
    pub fn next_after(&self, after: NaiveDateTime) -> Option<CalendarFire> {
        let first = after.date();
        (0..LOOKAHEAD_DAYS)
            .map(|offset| first + Duration::days(offset))
            .filter(|date| self.scheduled(*date))
            .find_map(|date| {
                let duty = self.duty_on(date);
                if duty.is_none() && self.names_people() {
                    return None; // nobody is responsible that day
                }
                let at = self
                    .times
                    .iter()
                    .map(|time| date.and_time(*time))
                    .find(|at| *at > after)?;
                Some(CalendarFire { at, duty })
            })
    }
}

/// Whether a calendar `person` is one of `names` (trimmed, any case).
pub fn is_one_of(person: &str, names: &[String]) -> bool {
    names
        .iter()
        .any(|name| name.trim().eq_ignore_ascii_case(person.trim()))
}

/// One scheduled fire of one trigger, the same on every machine that has
/// the trigger: `<trigger id>/<YYYY-MM-DD>/<HH:MM>` (local wall-clock).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DutyId {
    pub trigger_id: String,
    pub at: NaiveDateTime,
}

impl DutyId {
    pub fn parse(text: &str) -> Option<Self> {
        let (rest, time) = text.rsplit_once('/')?;
        let (trigger_id, date) = rest.rsplit_once('/')?;
        let at = NaiveDateTime::parse_from_str(&format!("{date} {time}"), "%Y-%m-%d %H:%M").ok()?;
        (!trigger_id.is_empty()).then(|| Self {
            trigger_id: trigger_id.to_string(),
            at,
        })
    }
}

impl std::fmt::Display for DutyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/{}",
            self.trigger_id,
            self.at.format("%Y-%m-%d/%H:%M")
        )
    }
}

/// `times` as `"HH:MM"` strings.
mod hhmm {
    use chrono::NaiveTime;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(times: &[NaiveTime], s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(times.iter().map(|t| t.format("%H:%M").to_string()))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<NaiveTime>, D::Error> {
        Vec::<String>::deserialize(d)?
            .iter()
            .map(|text| {
                NaiveTime::parse_from_str(text.trim(), "%H:%M").map_err(|_| {
                    serde::de::Error::custom(format!("'{text}' is not a time like 09:00"))
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn date(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    fn at(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap()
    }

    fn spec(config: serde_json::Value) -> CalendarSpec {
        CalendarSpec::from_value(config).unwrap()
    }

    fn person(spec: &CalendarSpec, day: &str) -> Option<String> {
        spec.duty_on(date(day)).map(|d| d.person)
    }

    #[test]
    fn fires_at_each_time_on_scheduled_days() {
        let s = spec(json!({"times": ["17:00", "09:00"], "days": ["mon", "fri"]}));
        // Wed 2026-10-28 08:00 → Fri 30th 09:00, then 17:00, then Mon 2nd
        let first = s.next_after(at("2026-10-28 08:00")).unwrap();
        assert_eq!(first.at, at("2026-10-30 09:00"));
        assert_eq!(first.duty, None);
        assert_eq!(s.next_after(first.at).unwrap().at, at("2026-10-30 17:00"));
        assert_eq!(
            s.next_after(at("2026-10-30 17:00")).unwrap().at,
            at("2026-11-02 09:00")
        );
    }

    #[test]
    fn skips_holidays() {
        let s = spec(json!({"times": ["09:00"], "skipDates": ["2026-12-25"]}));
        let next = s.next_after(at("2026-12-24 10:00")).unwrap();
        assert_eq!(next.at, at("2026-12-26 09:00"));
    }

    #[test]
    fn people_take_weekly_turns() {
        let s = spec(json!({
            "times": ["09:00"], "people": ["alice", "bob"], "startDate": "2026-10-26"
        }));
        assert_eq!(person(&s, "2026-10-26").as_deref(), Some("alice"));
        assert_eq!(person(&s, "2026-11-01").as_deref(), Some("alice"));
        assert_eq!(person(&s, "2026-11-02").as_deref(), Some("bob"));
        assert_eq!(person(&s, "2026-11-09").as_deref(), Some("alice"));
        assert_eq!(person(&s, "2026-10-25"), None, "before the rota starts");
    }

    #[test]
    fn daily_turns_skip_days_off() {
        let s = spec(json!({
            "times": ["09:00"], "days": ["mon", "tue", "wed", "thu", "fri"],
            "skipDates": ["2026-10-28"],
            "people": ["a", "b", "c"], "rotate": "day", "startDate": "2026-10-26"
        }));
        // Mon a, Tue b, (Wed holiday), Thu c, Fri a, (weekend), Mon b
        let who: Vec<_> = [
            "2026-10-26",
            "2026-10-27",
            "2026-10-29",
            "2026-10-30",
            "2026-11-02",
        ]
        .iter()
        .map(|d| person(&s, d).unwrap())
        .collect();
        assert_eq!(who, ["a", "b", "c", "a", "b"]);
    }

    #[test]
    fn fixed_days_beat_turns_and_covers_beat_both() {
        let s = spec(json!({
            "times": ["09:00"], "people": ["alice", "bob"], "startDate": "2026-10-26",
            "byDay": {"fri": "carol"},
            "cover": [{"person": "dana", "from": "2026-10-27", "to": "2026-10-30"}]
        }));
        let monday = s.duty_on(date("2026-10-26")).unwrap();
        assert_eq!(
            (monday.person.as_str(), monday.reason),
            ("alice", DutyReason::Rotation)
        );
        let tuesday = s.duty_on(date("2026-10-27")).unwrap();
        assert_eq!(tuesday.person, "dana");
        assert_eq!(tuesday.reason, DutyReason::Cover);
        assert_eq!(tuesday.covering_for.as_deref(), Some("alice"));
        let friday = s.duty_on(date("2026-10-30")).unwrap();
        assert_eq!(friday.covering_for.as_deref(), Some("carol"));
        let next_friday = s.duty_on(date("2026-11-06")).unwrap();
        assert_eq!(
            (next_friday.person.as_str(), next_friday.reason),
            ("carol", DutyReason::ByDay)
        );
    }

    #[test]
    fn each_day_has_a_backup() {
        let s = spec(json!({
            "times": ["09:00"], "people": ["alice", "bob", "carol"], "startDate": "2026-10-26",
            "byDay": {"sun": "bob"},
            "cover": [
                {"person": "dana", "from": "2026-11-02", "to": "2026-11-02", "backup": "erin"},
                {"person": "dana", "from": "2026-11-03", "to": "2026-11-03"}
            ]
        }));
        let backup = |day: &str| s.duty_on(date(day)).unwrap().backup;
        // alice's week: bob's turn is next
        assert_eq!(backup("2026-10-27").as_deref(), Some("bob"));
        // Sunday is bob's, so the backup is the next one who isn't bob
        assert_eq!(backup("2026-11-01").as_deref(), Some("carol"));
        // A cover's own backup, else the next turn
        assert_eq!(backup("2026-11-02").as_deref(), Some("erin"));
        assert_eq!(backup("2026-11-03").as_deref(), Some("carol"));

        let fixed = spec(json!({
            "times": ["09:00"], "byDay": {"mon": "alice", "tue": "erin"}, "backup": "erin"
        }));
        assert_eq!(
            fixed.duty_on(date("2026-10-26")).unwrap().backup.as_deref(),
            Some("erin")
        );
        // Never your own backup
        assert_eq!(fixed.duty_on(date("2026-10-27")).unwrap().backup, None);
    }

    #[test]
    fn days_without_anyone_responsible_are_skipped() {
        let s = spec(json!({"times": ["09:00"], "byDay": {"mon": "alice", "thu": "bob"}}));
        let next = s.next_after(at("2026-10-27 00:00")).unwrap();
        assert_eq!(next.at, at("2026-10-29 09:00"));
        assert_eq!(next.duty.unwrap().person, "bob");
    }

    #[test]
    fn rejects_bad_calendars() {
        for (config, expected) in [
            (json!({"times": []}), "at least one time"),
            (json!({"times": ["9am"]}), "not a time"),
            (json!({"times": ["09:00"], "days": ["funday"]}), "funday"),
            (json!({"times": ["09:00"], "people": ["a"]}), "startDate"),
            (json!({"times": ["09:00"], "byDay": {"mon": " "}}), "people"),
            (
                json!({"times": ["09:00"], "cover": [{"person": "d", "from": "2026-11-02", "to": "2026-11-01"}]}),
                "ends before",
            ),
            (json!({"times": ["09:00"], "typo": 1}), "unknown field"),
            (
                json!({"times": ["09:00"], "backupAfterMinutes": 0}),
                "at least 1",
            ),
            (json!({"times": ["09:00"], "backup": ""}), "people"),
        ] {
            let error = CalendarSpec::from_value(config.clone())
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{config}: {error}");
        }
    }

    #[test]
    fn knows_who_it_names() {
        let s = spec(json!({"times": ["09:00"], "byDay": {"mon": "Alice"}, "backup": "bob"}));
        assert!(s.mentions(&["alice-laptop-1".into(), " alice ".into()]));
        assert!(s.mentions(&["BOB".into()]));
        assert!(!s.mentions(&["carol".into()]));
    }

    #[test]
    fn duty_ids_round_trip() {
        let id = DutyId {
            trigger_id: "on-call".into(),
            at: at("2026-10-27 09:00"),
        };
        assert_eq!(id.to_string(), "on-call/2026-10-27/09:00");
        assert_eq!(DutyId::parse(&id.to_string()), Some(id));
        assert_eq!(DutyId::parse("on-call/2026-10-27"), None);
        assert_eq!(DutyId::parse("/2026-10-27/09:00"), None);
    }

    #[test]
    fn round_trips_as_json() {
        let s = spec(json!({
            "times": ["09:00"], "days": ["mon"], "people": ["a", "b"],
            "rotate": "day", "startDate": "2026-10-26", "byDay": {"fri": "c"}
        }));
        let value = serde_json::to_value(&s).unwrap();
        assert_eq!(value["times"], json!(["09:00"]));
        assert_eq!(value["startDate"], "2026-10-26");
        assert_eq!(CalendarSpec::from_value(value).unwrap(), s);
    }
}
