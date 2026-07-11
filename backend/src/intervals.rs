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

#[derive(Debug, Clone, PartialEq)]
pub struct ConsolidatedInterval {
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
    /// Sorted, deduplicated names of the sources active over this piece.
    pub sources: Vec<String>,
}

/// Merges per-source intervals into the cross-source awake-evidence view
/// (`consolidate=true`, spec 2026-07-10 API section, added 2026-07-11):
/// the union of ACTIVE time only, split wherever the set of active sources
/// changes, so `sources` is exact for every piece. Idle input is not
/// evidence of anything and is ignored entirely; time covered by no active
/// interval stays absent, so no-signal gaps remain gaps.
///
/// Adjacent pieces (sharing a boundary instant) merge when their source
/// sets are equal - the spec splits only where the set CHANGES. A
/// zero-length interval (single isolated sample) survives as a zero-length
/// piece when no covered time adjoins it; when covered time adjoins, its
/// instant already lies inside the neighboring piece. Known blind spot,
/// accepted: a zero-length interval landing exactly inside another
/// source's span does not add its name to that span's sources (it would
/// have to split a piece in two at a single instant to do so).
pub fn consolidate(per_source: &[(String, Vec<Interval>)]) -> Vec<ConsolidatedInterval> {
    let actives: Vec<(&str, &Interval)> = per_source
        .iter()
        .flat_map(|(source, ivs)| {
            ivs.iter()
                .filter(|iv| iv.state == State::Active)
                .map(move |iv| (source.as_str(), iv))
        })
        .collect();

    // Boundary instants of all active intervals, deduplicated by instant.
    // chrono compares DateTimes as instants regardless of offset; the
    // offset tiebreak below makes the dedup survivor - whose offset string
    // the response will render - deterministic when two sources report the
    // same instant with different UTC offsets.
    let mut bounds: Vec<DateTime<FixedOffset>> = actives
        .iter()
        .flat_map(|(_, iv)| [iv.start, iv.end])
        .collect();
    bounds.sort_by_key(|t| {
        (
            t.timestamp(),
            t.timestamp_subsec_nanos(),
            t.offset().local_minus_utc(),
        )
    });
    bounds.dedup();

    // The active sources covering the closed span [from, to]. Between two
    // adjacent bounds no interval starts or ends, so this is constant over
    // each elementary segment.
    let sources_covering = |from: DateTime<FixedOffset>, to: DateTime<FixedOffset>| {
        let mut sources: Vec<String> = actives
            .iter()
            .filter(|(_, iv)| iv.start <= from && iv.end >= to)
            .map(|(source, _)| source.to_string())
            .collect();
        sources.sort();
        sources.dedup();
        sources
    };

    let mut out: Vec<ConsolidatedInterval> = Vec::new();
    for (i, &bound) in bounds.iter().enumerate() {
        // Emit an isolated point piece only when neither neighboring
        // segment is covered; covered neighbors already contain the instant
        // as their closed endpoint.
        let covered_before = i > 0 && !sources_covering(bounds[i - 1], bound).is_empty();
        let covered_after =
            i + 1 < bounds.len() && !sources_covering(bound, bounds[i + 1]).is_empty();
        if !covered_before && !covered_after {
            let at_point = sources_covering(bound, bound);
            if !at_point.is_empty() {
                out.push(ConsolidatedInterval {
                    start: bound,
                    end: bound,
                    sources: at_point,
                });
            }
        }

        // The elementary segment from this bound to the next.
        let Some(&next) = bounds.get(i + 1) else {
            continue;
        };
        let sources = sources_covering(bound, next);
        if sources.is_empty() {
            continue;
        }
        match out.last_mut() {
            // Extends the previous piece: touching, same set.
            Some(last) if last.end == bound && last.sources == sources => last.end = next,
            _ => out.push(ConsolidatedInterval {
                start: bound,
                end: next,
                sources,
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

    // ----- consolidate -----

    fn iv(start: &str, end: &str, state: State) -> Interval {
        Interval {
            start: t(start),
            end: t(end),
            state,
        }
    }

    fn src(name: &str, ivs: Vec<Interval>) -> (String, Vec<Interval>) {
        (name.to_string(), ivs)
    }

    fn civ(start: &str, end: &str, sources: &[&str]) -> ConsolidatedInterval {
        ConsolidatedInterval {
            start: t(start),
            end: t(end),
            sources: sources.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn consolidate_empty_input_gives_nothing() {
        assert_eq!(consolidate(&[]), vec![]);
    }

    #[test]
    fn consolidate_single_source_single_interval_passes_through() {
        let got = consolidate(&[src(
            "macbook",
            vec![iv(
                "2026-07-10T22:00:00+03:00",
                "2026-07-10T22:05:00+03:00",
                State::Active,
            )],
        )]);
        assert_eq!(
            got,
            vec![civ(
                "2026-07-10T22:00:00+03:00",
                "2026-07-10T22:05:00+03:00",
                &["macbook"]
            )]
        );
    }

    #[test]
    fn consolidate_ignores_idle_entirely() {
        // idle is not awake-evidence; a source that is only idle contributes
        // nothing, not even to the sources list of overlapping active time
        let got = consolidate(&[
            src(
                "macbook",
                vec![iv(
                    "2026-07-10T22:00:00+03:00",
                    "2026-07-10T22:05:00+03:00",
                    State::Idle,
                )],
            ),
            src(
                "pixel",
                vec![iv(
                    "2026-07-10T22:00:00+03:00",
                    "2026-07-10T22:05:00+03:00",
                    State::Active,
                )],
            ),
        ]);
        assert_eq!(
            got,
            vec![civ(
                "2026-07-10T22:00:00+03:00",
                "2026-07-10T22:05:00+03:00",
                &["pixel"]
            )]
        );
    }

    #[test]
    fn consolidate_splits_where_the_active_set_changes() {
        // mac 22:00-22:10, pixel 22:05-22:15: three pieces, exact sources each
        let got = consolidate(&[
            src(
                "macbook",
                vec![iv(
                    "2026-07-10T22:00:00+03:00",
                    "2026-07-10T22:10:00+03:00",
                    State::Active,
                )],
            ),
            src(
                "pixel",
                vec![iv(
                    "2026-07-10T22:05:00+03:00",
                    "2026-07-10T22:15:00+03:00",
                    State::Active,
                )],
            ),
        ]);
        assert_eq!(
            got,
            vec![
                civ(
                    "2026-07-10T22:00:00+03:00",
                    "2026-07-10T22:05:00+03:00",
                    &["macbook"]
                ),
                civ(
                    "2026-07-10T22:05:00+03:00",
                    "2026-07-10T22:10:00+03:00",
                    &["macbook", "pixel"]
                ),
                civ(
                    "2026-07-10T22:10:00+03:00",
                    "2026-07-10T22:15:00+03:00",
                    &["pixel"]
                ),
            ]
        );
    }

    #[test]
    fn consolidate_identical_spans_merge_with_sorted_sources() {
        // input deliberately in reverse-alphabetical source order
        let span = |st| iv("2026-07-10T22:00:00+03:00", "2026-07-10T22:10:00+03:00", st);
        let got = consolidate(&[
            src("pixel", vec![span(State::Active)]),
            src("macbook", vec![span(State::Active)]),
        ]);
        assert_eq!(
            got,
            vec![civ(
                "2026-07-10T22:00:00+03:00",
                "2026-07-10T22:10:00+03:00",
                &["macbook", "pixel"]
            )]
        );
    }

    #[test]
    fn consolidate_preserves_gaps_between_intervals() {
        let got = consolidate(&[src(
            "macbook",
            vec![
                iv(
                    "2026-07-10T22:00:00+03:00",
                    "2026-07-10T22:05:00+03:00",
                    State::Active,
                ),
                iv(
                    "2026-07-10T22:20:00+03:00",
                    "2026-07-10T22:25:00+03:00",
                    State::Active,
                ),
            ],
        )]);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].end, t("2026-07-10T22:05:00+03:00"));
        assert_eq!(got[1].start, t("2026-07-10T22:20:00+03:00"));
    }

    #[test]
    fn consolidate_merges_adjacent_pieces_with_equal_sets() {
        // touching intervals with the same source set do not split: the spec
        // says split only where the set CHANGES
        let got = consolidate(&[src(
            "macbook",
            vec![
                iv(
                    "2026-07-10T22:00:00+03:00",
                    "2026-07-10T22:05:00+03:00",
                    State::Active,
                ),
                iv(
                    "2026-07-10T22:05:00+03:00",
                    "2026-07-10T22:10:00+03:00",
                    State::Active,
                ),
            ],
        )]);
        assert_eq!(
            got,
            vec![civ(
                "2026-07-10T22:00:00+03:00",
                "2026-07-10T22:10:00+03:00",
                &["macbook"]
            )]
        );
    }

    #[test]
    fn consolidate_isolated_zero_length_interval_survives() {
        // a single 3am phone sample is real awake-evidence
        let got = consolidate(&[
            src(
                "macbook",
                vec![iv(
                    "2026-07-10T22:00:00+03:00",
                    "2026-07-10T22:05:00+03:00",
                    State::Active,
                )],
            ),
            src(
                "pixel",
                vec![iv(
                    "2026-07-11T03:00:00+03:00",
                    "2026-07-11T03:00:00+03:00",
                    State::Active,
                )],
            ),
        ]);
        assert_eq!(
            got,
            vec![
                civ(
                    "2026-07-10T22:00:00+03:00",
                    "2026-07-10T22:05:00+03:00",
                    &["macbook"]
                ),
                civ(
                    "2026-07-11T03:00:00+03:00",
                    "2026-07-11T03:00:00+03:00",
                    &["pixel"]
                ),
            ]
        );
    }

    #[test]
    fn consolidate_zero_length_inside_covered_time_is_absorbed() {
        // the pixel instant lies inside mac's active span; its instant is
        // already covered, so no extra piece and no split
        let got = consolidate(&[
            src(
                "macbook",
                vec![iv(
                    "2026-07-10T22:00:00+03:00",
                    "2026-07-10T22:10:00+03:00",
                    State::Active,
                )],
            ),
            src(
                "pixel",
                vec![iv(
                    "2026-07-10T22:05:00+03:00",
                    "2026-07-10T22:05:00+03:00",
                    State::Active,
                )],
            ),
        ]);
        assert_eq!(
            got,
            vec![civ(
                "2026-07-10T22:00:00+03:00",
                "2026-07-10T22:10:00+03:00",
                &["macbook"]
            )]
        );
    }

    #[test]
    fn consolidate_zero_length_at_anothers_boundary_is_absorbed_without_attribution() {
        // Pins the documented blind spot (see the consolidate doc comment):
        // a zero-length interval coinciding with another source's boundary
        // instant stays covered but unattributed. If a refactor changes
        // this in either direction, this test must be revisited on purpose.
        let got = consolidate(&[
            src(
                "macbook",
                vec![iv(
                    "2026-07-10T22:00:00+03:00",
                    "2026-07-10T22:10:00+03:00",
                    State::Active,
                )],
            ),
            src(
                "pixel",
                vec![iv(
                    "2026-07-10T22:10:00+03:00",
                    "2026-07-10T22:10:00+03:00",
                    State::Active,
                )],
            ),
        ]);
        assert_eq!(
            got,
            vec![civ(
                "2026-07-10T22:00:00+03:00",
                "2026-07-10T22:10:00+03:00",
                &["macbook"]
            )]
        );
    }

    #[test]
    fn consolidate_mixed_offsets_compare_as_instants() {
        // 19:00Z and 22:00+03:00 are the same instant: one merged piece
        let got = consolidate(&[
            src(
                "macbook",
                vec![iv(
                    "2026-07-10T19:00:00Z",
                    "2026-07-10T19:05:00Z",
                    State::Active,
                )],
            ),
            src(
                "pixel",
                vec![iv(
                    "2026-07-10T22:00:00+03:00",
                    "2026-07-10T22:05:00+03:00",
                    State::Active,
                )],
            ),
        ]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].sources, vec!["macbook", "pixel"]);
    }
}
