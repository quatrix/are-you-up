use chrono::{DateTime, FixedOffset};

/// Samples further apart than this cannot belong to the same interval: the
/// tracker was off or asleep in between, and that time must stay no-signal.
/// 3x the client sample period (30s).
pub const MAX_GAP_S: i64 = 90;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Active,
    Idle,
}

#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub t: DateTime<FixedOffset>,
    pub idle_s: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Interval {
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
    pub state: State,
}

/// Turns time-sorted samples into merged intervals. See module tests for
/// the exact semantics of the threshold and the gap break.
///
/// Precondition: `samples` must be sorted ascending by `t`. Unsorted input
/// silently produces meaningless intervals - there is no error to return,
/// just wrong output - so callers must sort before calling this.
pub fn derive(samples: &[Sample], threshold_s: i64, max_gap_s: i64) -> Vec<Interval> {
    let mut out: Vec<Interval> = Vec::new();
    for sample in samples {
        let state = if sample.idle_s < threshold_s {
            State::Active
        } else {
            State::Idle
        };
        match out.last_mut() {
            // Extend the current run: same state, and close enough in time.
            // num_seconds() truncates toward zero, so a sub-second overshoot
            // past max_gap_s still merges here - deliberate, since 90s is a
            // loose heuristic already; do not "fix" the rounding either way.
            Some(last)
                if last.state == state && (sample.t - last.end).num_seconds() <= max_gap_s =>
            {
                debug_assert!(
                    last.end <= sample.t,
                    "samples must be sorted ascending by t; got {:?} after {:?}",
                    sample.t,
                    last.end
                );
                last.end = sample.t;
            }
            // State flip or gap break: start a new interval at this sample.
            _ => out.push(Interval {
                start: sample.t,
                end: sample.t,
                state,
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(s).expect("test literals are valid RFC 3339")
    }

    fn s(ts: &str, idle_s: i64) -> Sample {
        Sample { t: t(ts), idle_s }
    }

    #[test]
    fn empty_input_gives_no_intervals() {
        assert_eq!(derive(&[], 900, MAX_GAP_S), vec![]);
    }

    #[test]
    fn single_sample_is_a_point_interval() {
        let got = derive(&[s("2026-07-10T22:00:00+03:00", 5)], 900, MAX_GAP_S);
        assert_eq!(
            got,
            vec![Interval {
                start: t("2026-07-10T22:00:00+03:00"),
                end: t("2026-07-10T22:00:00+03:00"),
                state: State::Active
            }]
        );
    }

    #[test]
    fn threshold_boundary_is_idle() {
        // idle_s == threshold means the last input was exactly threshold ago:
        // that is NOT "within the last threshold seconds", so it is idle.
        let got = derive(&[s("2026-07-10T22:00:00+03:00", 900)], 900, MAX_GAP_S);
        assert_eq!(got[0].state, State::Idle);
        let got = derive(&[s("2026-07-10T22:00:00+03:00", 899)], 900, MAX_GAP_S);
        assert_eq!(got[0].state, State::Active);
    }

    #[test]
    fn same_state_samples_within_gap_merge() {
        let got = derive(
            &[
                s("2026-07-10T22:00:00+03:00", 1),
                s("2026-07-10T22:00:30+03:00", 2),
                s("2026-07-10T22:01:00+03:00", 3),
            ],
            900,
            MAX_GAP_S,
        );
        assert_eq!(
            got,
            vec![Interval {
                start: t("2026-07-10T22:00:00+03:00"),
                end: t("2026-07-10T22:01:00+03:00"),
                state: State::Active
            }]
        );
    }

    #[test]
    fn state_change_splits_intervals() {
        let got = derive(
            &[
                s("2026-07-10T22:00:00+03:00", 1),
                s("2026-07-10T22:00:30+03:00", 1000),
                s("2026-07-10T22:01:00+03:00", 1030),
            ],
            900,
            MAX_GAP_S,
        );
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].state, State::Active);
        assert_eq!(got[0].end, t("2026-07-10T22:00:00+03:00"));
        assert_eq!(got[1].state, State::Idle);
        assert_eq!(got[1].start, t("2026-07-10T22:00:30+03:00"));
        assert_eq!(got[1].end, t("2026-07-10T22:01:00+03:00"));
    }

    #[test]
    fn gap_over_max_splits_even_with_same_state() {
        let got = derive(
            &[
                s("2026-07-10T22:00:00+03:00", 1),
                s("2026-07-10T22:00:30+03:00", 2),
                // 91s after the previous sample: one over the limit
                s("2026-07-10T22:02:01+03:00", 3),
            ],
            900,
            MAX_GAP_S,
        );
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].end, t("2026-07-10T22:00:30+03:00"));
        assert_eq!(got[1].start, t("2026-07-10T22:02:01+03:00"));
    }

    #[test]
    fn gap_of_exactly_max_still_merges() {
        let got = derive(
            &[
                s("2026-07-10T22:00:00+03:00", 1),
                s("2026-07-10T22:01:30+03:00", 2),
            ],
            900,
            MAX_GAP_S,
        );
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn mixed_offsets_compare_as_instants() {
        // 20:00:00Z and 23:00:30+03:00 are 30 seconds apart in real time.
        let got = derive(
            &[
                s("2026-07-10T20:00:00Z", 1),
                s("2026-07-10T23:00:30+03:00", 2),
            ],
            900,
            MAX_GAP_S,
        );
        assert_eq!(got.len(), 1);
    }
}
