//! Weekly quota budgeting over explicit local working intervals.
use crate::{
    instances::ProviderId,
    limits::{LimitWindow, PaceTip},
    settings::ProviderKind,
};
use chrono::{DateTime, Datelike, Duration, Local, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::{LazyLock, RwLock},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkSchedule {
    pub enabled: bool,
    /// Monday = 0, Sunday = 6.
    pub weekdays: Vec<u8>,
    pub start_minutes: u16,
    pub end_minutes: u16,
}
impl Default for WorkSchedule {
    fn default() -> Self {
        Self {
            enabled: false,
            weekdays: vec![0, 1, 2, 3, 4],
            start_minutes: 540,
            end_minutes: 1020,
        }
    }
}
impl WorkSchedule {
    pub fn valid(&self) -> bool {
        !self.weekdays.is_empty()
            && self.weekdays.iter().all(|day| *day < 7)
            && self.start_minutes < self.end_minutes
            && self.end_minutes < 1440
    }
    pub fn summary(&self) -> String {
        let names = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        let days = self
            .weekdays
            .iter()
            .filter_map(|day| names.get(*day as usize))
            .copied()
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{} {:02}:{:02}–{:02}:{:02} · local time",
            days,
            self.start_minutes / 60,
            self.start_minutes % 60,
            self.end_minutes / 60,
            self.end_minutes % 60
        )
    }
}
static SCHEDULE: LazyLock<RwLock<WorkSchedule>> =
    LazyLock::new(|| RwLock::new(WorkSchedule::default()));
static FAILED: LazyLock<RwLock<HashSet<ProviderId>>> =
    LazyLock::new(|| RwLock::new(HashSet::new()));
static SAMPLES: LazyLock<RwLock<HashMap<ProviderId, DateTime<Utc>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
pub fn record_sample(provider: ProviderId, sampled_at: DateTime<Utc>) {
    SAMPLES
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(provider, sampled_at);
    set_failed(provider, false);
}
pub fn publish(schedule: &WorkSchedule) {
    *SCHEDULE.write().unwrap_or_else(|e| e.into_inner()) = schedule.clone();
}
pub fn current() -> WorkSchedule {
    SCHEDULE.read().unwrap_or_else(|e| e.into_inner()).clone()
}
pub fn set_failed(provider: ProviderId, failed: bool) {
    let mut providers = FAILED.write().unwrap_or_else(|e| e.into_inner());
    if failed {
        providers.insert(provider);
    } else {
        providers.remove(&provider);
    }
}
static INTERVALS: LazyLock<RwLock<HashMap<ProviderId, i64>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
pub fn publish_intervals(instances: &[crate::settings::ProviderInstance]) {
    *INTERVALS.write().unwrap_or_else(|e| e.into_inner()) = instances
        .iter()
        .map(|instance| {
            (
                instance.provider_id(),
                instance.refresh_interval().seconds() as i64,
            )
        })
        .collect();
}
pub fn is_fresh(provider: ProviderId, sampled_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    !FAILED
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&provider)
        && sampled_at <= now + Duration::minutes(1)
        && now - sampled_at
            <= Duration::seconds(
                INTERVALS
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&provider)
                    .map_or(35 * 60, |seconds| seconds * 2 + 120),
            )
}

/// Split by local calendar dates, resolving offsets separately for every boundary.
/// Ambiguous times use the earlier occurrence for starts, later for ends. A
/// missing spring-forward boundary advances to the first real local minute.
fn boundary<T: TimeZone>(
    zone: &T,
    date: chrono::NaiveDate,
    minutes: u16,
    end: bool,
) -> Option<DateTime<Utc>> {
    let local = date.and_hms_opt(u32::from(minutes / 60), u32::from(minutes % 60), 0)?;
    for shift in 0..=180 {
        let result = zone.from_local_datetime(&(local + Duration::minutes(shift)));
        let resolved = if end {
            result.latest()
        } else {
            result.earliest()
        };
        if let Some(at) = resolved {
            return Some(at.with_timezone(&Utc));
        }
    }
    None
}
fn work_seconds<T: TimeZone>(
    schedule: &WorkSchedule,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    zone: &T,
) -> Option<f64> {
    if end <= start {
        return Some(0.0);
    }
    let mut day = start.with_timezone(zone).date_naive();
    let last = end.with_timezone(zone).date_naive();
    let mut seconds = 0.0;
    while day <= last {
        if schedule
            .weekdays
            .contains(&(day.weekday().num_days_from_monday() as u8))
        {
            let from = boundary(zone, day, schedule.start_minutes, false)?.max(start);
            let until = boundary(zone, day, schedule.end_minutes, true)?.min(end);
            seconds += (until - from).num_milliseconds().max(0) as f64 / 1000.0;
        }
        day = day.succ_opt()?;
    }
    Some(seconds)
}
pub fn expected<T: TimeZone>(
    schedule: &WorkSchedule,
    window: &LimitWindow,
    now: DateTime<Utc>,
    zone: &T,
) -> Result<f64, &'static str> {
    if !schedule.valid() {
        return Err("Invalid work schedule");
    }
    let reset = window.resets_at.ok_or("Reset time unavailable")?;
    let minutes = window
        .duration_minutes
        .filter(|minutes| *minutes > 0)
        .ok_or("Window duration unavailable")?;
    // Bound provider input to avoid unbounded calendar iteration or overflow.
    if minutes > 366 * 1440 {
        return Err("Unsupported quota window duration");
    }
    let start = reset
        .checked_sub_signed(Duration::minutes(i64::from(minutes)))
        .ok_or("Invalid quota window")?;
    if now >= reset {
        return Err("Reset passed; awaiting updated quota");
    }
    if now < start {
        return Err("Quota window has not started");
    }
    let total =
        work_seconds(schedule, start, reset, zone).ok_or("Cannot resolve local work hours")?;
    if total <= 0.0 {
        return Err("No scheduled work in this quota window");
    }
    let elapsed =
        work_seconds(schedule, start, now, zone).ok_or("Cannot resolve local work hours")?;
    Ok((100.0 * elapsed / total).clamp(0.0, 100.0))
}
pub fn reserve<T: TimeZone>(
    schedule: &WorkSchedule,
    window: &LimitWindow,
    now: DateTime<Utc>,
    zone: &T,
) -> Result<f64, &'static str> {
    let actual = window.used_percent.ok_or("Usage unavailable")?;
    Ok(expected(schedule, window, now, zone)? - f64::from(actual.min(100)))
}
pub fn signed(value: f64) -> String {
    let rounded = value.round() as i32;
    if rounded == 0 {
        "0".into()
    } else {
        format!("{rounded:+}")
    }
}
pub fn tip(
    provider: ProviderId,
    weekly: bool,
    window: &LimitWindow,
    show_used: bool,
    now: DateTime<Utc>,
) -> Option<PaceTip> {
    tip_with_schedule(provider, weekly, window, show_used, now, &current())
}
fn tip_with_schedule(
    provider: ProviderId,
    weekly: bool,
    window: &LimitWindow,
    show_used: bool,
    now: DateTime<Utc>,
    schedule: &WorkSchedule,
) -> Option<PaceTip> {
    if provider.kind() != ProviderKind::Codex || !weekly || !schedule.enabled {
        return window.pace_tip(show_used, now);
    }
    if FAILED
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&provider)
    {
        return None;
    }
    let expected = expected(schedule, window, now, &Local).ok()?;
    Some(PaceTip {
        percent: if show_used {
            expected
        } else {
            100.0 - expected
        },
        delta_percent: f64::from(window.used_percent?) - expected,
    })
}
pub fn card_tip(
    key: &str,
    window: &LimitWindow,
    show_used: bool,
    now: DateTime<Utc>,
) -> Option<PaceTip> {
    let Some((id, metric)) = key.split_once(':') else {
        return window.pace_tip(show_used, now);
    };
    let Some(provider) = ProviderId::lookup(id) else {
        return window.pace_tip(show_used, now);
    };
    if provider.kind() == ProviderKind::Codex && metric == "weekly" && current().enabled {
        let sampled_at = SAMPLES
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&provider)
            .copied()?;
        if !is_fresh(provider, sampled_at, now) {
            return None;
        }
    }
    tip(provider, metric == "weekly", window, show_used, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, LocalResult, NaiveDate, NaiveDateTime};
    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, day, hour, 0, 0).unwrap()
    }
    fn window(used: u8) -> LimitWindow {
        LimitWindow {
            used_percent: Some(used),
            resets_at: Some(at(19, 9)),
            duration_minutes: Some(10080),
        }
    }
    fn schedule() -> WorkSchedule {
        WorkSchedule {
            enabled: true,
            ..Default::default()
        }
    }
    #[test]
    fn exact_reserve_deficit_and_zero() {
        // Monday 09:00 through next Monday 09:00; Wednesday 09:00 is 16/40 hours.
        assert!((expected(&schedule(), &window(30), at(14, 9), &Utc).unwrap() - 40.0).abs() < 1e-9);
        for (used, result) in [(30, 10.0), (50, -10.0), (40, 0.0)] {
            assert_eq!(
                reserve(&schedule(), &window(used), at(14, 9), &Utc),
                Ok(result)
            );
        }
    }
    #[test]
    fn non_working_time_is_flat_but_spending_still_counts() {
        let s = schedule();
        let w = window(30);
        for (from, to) in [(at(12, 17), at(13, 9)), (at(16, 17), at(19, 8))] {
            assert_eq!(expected(&s, &w, from, &Utc), expected(&s, &w, to, &Utc));
        }
        assert_eq!(reserve(&s, &window(40), at(17, 12), &Utc), Ok(60.0));
        assert_eq!(reserve(&s, &window(50), at(17, 12), &Utc), Ok(50.0));
    }
    #[test]
    fn weekend_toggle_and_schedule_edits_recalculate_entire_window() {
        let mut s = schedule();
        let weekday = expected(&s, &window(0), at(14, 9), &Utc).unwrap();
        s.weekdays.extend([5, 6]);
        assert!(expected(&s, &window(0), at(14, 9), &Utc).unwrap() < weekday);
        s.weekdays = vec![0, 1, 2, 3, 4];
        s.end_minutes = 12 * 60;
        assert_ne!(
            expected(&s, &window(0), at(14, 12), &Utc),
            expected(&schedule(), &window(0), at(14, 12), &Utc)
        );
    }
    #[test]
    fn partial_boundary_days_and_early_window() {
        let w = LimitWindow {
            resets_at: Some(at(19, 13)),
            ..window(0)
        };
        // Monday 13:00 start: 4h Monday + 32h Tue-Fri + 4h next Monday.
        assert_eq!(expected(&schedule(), &w, at(12, 13), &Utc), Ok(0.0));
        assert_eq!(expected(&schedule(), &w, at(12, 17), &Utc), Ok(10.0));
        assert_eq!(expected(&schedule(), &w, at(19, 12), &Utc), Ok(97.5));
    }
    #[test]
    fn invalid_or_missing_inputs_never_make_up_pace() {
        let mut s = schedule();
        let mut w = window(0);
        s.weekdays.clear();
        assert!(expected(&s, &w, at(14, 9), &Utc).is_err());
        s = schedule();
        s.end_minutes = s.start_minutes;
        assert!(expected(&s, &w, at(14, 9), &Utc).is_err());
        s = schedule();
        w.duration_minutes = None;
        assert!(expected(&s, &w, at(14, 9), &Utc).is_err());
        w = window(0);
        w.resets_at = None;
        assert!(expected(&s, &w, at(14, 9), &Utc).is_err());
        w = window(0);
        w.used_percent = None;
        assert!(reserve(&s, &w, at(14, 9), &Utc).is_err());
        w = window(0);
        assert!(expected(&s, &w, at(19, 9), &Utc).is_err());
        assert!(expected(&s, &w, at(12, 8), &Utc).is_err());
        w.resets_at = Some(at(18, 12));
        w.duration_minutes = Some(60);
        assert!(expected(&s, &w, at(18, 11), &Utc).is_err());
    }
    #[test]
    fn zero_and_signed_formatting() {
        assert_eq!(signed(10.2), "+10");
        assert_eq!(signed(-10.2), "-10");
        assert_eq!(signed(-0.2), "0");
        assert_eq!(signed(0.2), "0");
    }
    #[test]
    fn stale_age_and_failed_refresh_are_explicit() {
        let provider = ProviderId::primary(ProviderKind::Claude);
        let now = at(14, 9);
        assert!(is_fresh(provider, now, now));
        assert!(!is_fresh(provider, now - Duration::minutes(36), now));
        assert!(!is_fresh(provider, now + Duration::hours(2), now));
        // Use a separate identity so this test does not mutate other tests' runtime state.
        set_failed(provider, true);
        assert!(!is_fresh(provider, now, now));
        set_failed(provider, false);
    }

    // Deterministic US Central 2026 rules, independent of the host timezone.
    #[derive(Clone, Copy)]
    struct Central2026;
    impl TimeZone for Central2026 {
        type Offset = FixedOffset;
        fn from_offset(_: &FixedOffset) -> Self {
            Self
        }
        fn offset_from_local_date(&self, date: &NaiveDate) -> LocalResult<FixedOffset> {
            self.offset_from_local_datetime(&date.and_hms_opt(12, 0, 0).unwrap())
        }
        fn offset_from_utc_date(&self, date: &NaiveDate) -> FixedOffset {
            self.offset_from_utc_datetime(&date.and_hms_opt(12, 0, 0).unwrap())
        }
        fn offset_from_utc_datetime(&self, dt: &NaiveDateTime) -> FixedOffset {
            let spring = NaiveDate::from_ymd_opt(2026, 3, 8)
                .unwrap()
                .and_hms_opt(8, 0, 0)
                .unwrap();
            let fall = NaiveDate::from_ymd_opt(2026, 11, 1)
                .unwrap()
                .and_hms_opt(7, 0, 0)
                .unwrap();
            FixedOffset::west_opt(if *dt >= spring && *dt < fall {
                5 * 3600
            } else {
                6 * 3600
            })
            .unwrap()
        }
        fn offset_from_local_datetime(&self, dt: &NaiveDateTime) -> LocalResult<FixedOffset> {
            let mut matches = Vec::new();
            for hours in [5, 6] {
                let offset = FixedOffset::west_opt(hours * 3600).unwrap();
                if self.offset_from_utc_datetime(&(*dt + Duration::hours(i64::from(hours))))
                    == offset
                {
                    matches.push(offset);
                }
            }
            match matches.as_slice() {
                [one] => LocalResult::Single(*one),
                [one, two] => LocalResult::Ambiguous(*one, *two),
                _ => LocalResult::None,
            }
        }
    }
    #[test]
    fn daylight_saving_transitions_measure_real_work_intervals() {
        let mut s = schedule();
        s.weekdays = vec![6];
        s.start_minutes = 60;
        s.end_minutes = 240;
        for (month, day, hours) in [(3, 8, 2), (11, 1, 4)] {
            let from = Utc.with_ymd_and_hms(2026, month, day, 0, 0, 0).unwrap();
            assert_eq!(
                work_seconds(&s, from, from + Duration::days(1), &Central2026),
                Some(f64::from(hours) * 3600.0)
            );
        }
        // A nonexistent spring boundary at 02:30 advances to 03:00.
        let day = NaiveDate::from_ymd_opt(2026, 3, 8).unwrap();
        assert_eq!(
            boundary(&Central2026, day, 150, false),
            Some(Utc.with_ymd_and_hms(2026, 3, 8, 8, 0, 0).unwrap())
        );
        // A repeated fall boundary chooses the outer occurrences.
        let day = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        assert_eq!(
            boundary(&Central2026, day, 90, true).unwrap()
                - boundary(&Central2026, day, 90, false).unwrap(),
            Duration::hours(1)
        );
    }
    #[test]
    fn non_weekly_and_other_providers_preserve_original_pace() {
        let w = window(30);
        let now = at(14, 9);
        let s = schedule();
        for provider in [
            ProviderId::primary(ProviderKind::Codex),
            ProviderId::primary(ProviderKind::Claude),
        ] {
            for weekly in [false, true] {
                if provider.kind() == ProviderKind::Codex && weekly {
                    continue;
                }
                for show_used in [false, true] {
                    assert_eq!(
                        tip_with_schedule(provider, weekly, &w, show_used, now, &s),
                        w.pace_tip(show_used, now)
                    );
                }
            }
        }
        let disabled = WorkSchedule::default();
        assert_eq!(
            tip_with_schedule(
                ProviderId::primary(ProviderKind::Codex),
                true,
                &w,
                false,
                now,
                &disabled
            ),
            w.pace_tip(false, now)
        );
    }
}
