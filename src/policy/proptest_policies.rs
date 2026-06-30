use super::fixed_window::FixedWindowState;
use super::sliding_window::SlidingWindowState;
use super::token_bucket::TokenBucketState;
use super::PolicyState;
use crate::quota::Quota;
use proptest::prelude::*;

fn monotonic_offsets(max_events: usize, max_step_ms: u64) -> impl Strategy<Value = Vec<u64>> {
    prop::collection::vec(0u64..max_step_ms, 1..max_events).prop_map(|deltas| {
        let mut total = 0u64;
        deltas
            .iter()
            .map(|delta| {
                total = total.saturating_add(*delta);
                total
            })
            .collect()
    })
}

const BASE_MS: u64 = 10_000_000;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn token_bucket_remaining_within_limit(
        max in 1usize..30,
        burst in 1usize..50,
        offsets in monotonic_offsets(80, 400),
    ) {
        let burst = burst.max(max);
        let quota = Quota::with_burst(max, 1000, burst);
        let mut state = TokenBucketState::new_at(quota, BASE_MS);

        for offset in offsets {
            let snapshot = state.try_acquire(BASE_MS + offset);
            prop_assert!(snapshot.remaining <= snapshot.limit);
            prop_assert_eq!(snapshot.limit, burst);
        }
    }

    #[test]
    fn token_bucket_same_instant_respects_burst(
        max in 1usize..20,
        burst in 1usize..30,
        extra in 0usize..10,
    ) {
        let burst = burst.max(max);
        let quota = Quota::with_burst(max, 1000, burst);
        let mut state = TokenBucketState::new_at(quota, BASE_MS);

        let mut allowed = 0usize;
        for _ in 0..burst.saturating_add(extra) {
            if state.try_acquire(BASE_MS).allowed {
                allowed += 1;
            }
        }

        prop_assert_eq!(allowed, burst);
    }

    #[test]
    fn fixed_window_remaining_within_limit(
        max in 1usize..30,
        per_ms in 100u64..5000,
        offsets in monotonic_offsets(80, 200),
    ) {
        let quota = Quota::new(max, per_ms);
        let mut state = FixedWindowState::new_at(quota, BASE_MS);

        for offset in offsets {
            let snapshot = state.try_acquire(BASE_MS + offset);
            prop_assert!(snapshot.remaining <= snapshot.limit);
            prop_assert_eq!(snapshot.limit, max.max(1));
        }
    }

    #[test]
    fn fixed_window_same_window_never_exceeds_max(
        max in 1usize..20,
        per_ms in 200u64..2000,
        extra in 0usize..10,
    ) {
        let quota = Quota::new(max, per_ms);
        let mut state = FixedWindowState::new_at(quota, BASE_MS);

        let mut allowed = 0usize;
        for _ in 0..max.saturating_add(extra) {
            if state.try_acquire(BASE_MS).allowed {
                allowed += 1;
            }
        }

        prop_assert_eq!(allowed, max);
    }

    #[test]
    fn sliding_window_remaining_within_limit(
        max in 1usize..30,
        per_ms in 100u64..5000,
        offsets in monotonic_offsets(80, 200),
    ) {
        let quota = Quota::new(max, per_ms);
        let mut state = SlidingWindowState::new_at(quota, BASE_MS);

        for offset in offsets {
            let snapshot = state.try_acquire(BASE_MS + offset);
            prop_assert!(snapshot.remaining <= snapshot.limit);
            prop_assert_eq!(snapshot.limit, max.max(1));
        }
    }

    #[test]
    fn sliding_window_same_instant_respects_max(
        max in 1usize..20,
        per_ms in 200u64..2000,
        extra in 0usize..10,
    ) {
        let quota = Quota::new(max, per_ms);
        let mut state = SlidingWindowState::new_at(quota, BASE_MS);

        let mut allowed = 0usize;
        for _ in 0..max.saturating_add(extra) {
            if state.try_acquire(BASE_MS).allowed {
                allowed += 1;
            }
        }

        prop_assert_eq!(allowed, max);
    }

    #[test]
    fn fixed_window_never_exceeds_max_within_each_window(
        max in 1usize..20,
        per_ms in 200u64..2000,
        offsets in monotonic_offsets(80, 400),
    ) {
        let quota = Quota::new(max, per_ms);
        let mut state = FixedWindowState::new_at(quota, BASE_MS);
        let mut allowed_in_window = 0usize;
        let mut reset_at = 0u64;

        for offset in offsets {
            let snapshot = state.try_acquire(BASE_MS + offset);
            if reset_at != snapshot.reset_at_ms {
                allowed_in_window = 0;
                reset_at = snapshot.reset_at_ms;
            }
            if snapshot.allowed {
                allowed_in_window += 1;
            }
            prop_assert!(allowed_in_window <= max);
        }
    }

    #[test]
    fn sliding_window_never_exceeds_max_within_each_window(
        max in 1usize..20,
        per_ms in 200u64..2000,
        offsets in monotonic_offsets(80, 400),
    ) {
        let quota = Quota::new(max, per_ms);
        let mut state = SlidingWindowState::new_at(quota, BASE_MS);
        let mut allowed_in_window = 0usize;
        let mut reset_at = 0u64;

        for offset in offsets {
            let snapshot = state.try_acquire(BASE_MS + offset);
            if reset_at != snapshot.reset_at_ms {
                allowed_in_window = 0;
                reset_at = snapshot.reset_at_ms;
            }
            if snapshot.allowed {
                allowed_in_window += 1;
            }
            prop_assert!(allowed_in_window <= max);
        }
    }
}
