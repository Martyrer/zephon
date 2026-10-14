//! Suggests a more robust latency profile for a receiver from its recent network loss.
//!
//! The session measures loss from the receiver's retransmit requests on the RAOP control
//! channel (Nyx `raop::ap2::session::Ap2Stats`) and turns it into per-minute rates over the
//! last ~60 s ([`LossStats`]). The controller calls [`suggest_profile`] on every stats update
//! (every 5 s) and the receiver row shows `Glitchy? Try <profile>` when it returns `Some`.
//! Nothing switches automatically.

use std::time::Duration;

use crate::session::LatencyProfile;

/// How far back [`LossStats`] looks.
pub const LOSS_WINDOW: Duration = Duration::from_secs(60);

/// Recent network loss for one receiver session.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LossStats {
    /// The span the rates below were measured over: [`LOSS_WINDOW`] (~60 s) once the session
    /// has run that long, shorter right after (re)start. Startup loss weighs more while it is short.
    pub window: Duration,
    /// Retransmit requests the receiver sent per minute. Each request names a run of one
    /// or more lost packets; any value above zero means packets are being lost on the way.
    pub resend_requests_per_min: f64,
    /// Packets per minute the receiver asked for that could not be resent (no longer in the
    /// sender's ~4 s history, or never sent). These are unrecoverable: likely audible glitches.
    pub missed_per_min: f64,
}

/// Decide whether to suggest a more robust latency profile for a receiver,
/// based on its recent network loss stats. Returns the profile to suggest,
/// or None if the current one is fine.
#[allow(unused_variables)] // remove once the body uses its inputs
pub fn suggest_profile(current: LatencyProfile, stats: &LossStats) -> Option<LatencyProfile> {
    // TODO(user): decide thresholds. Inputs: resends/min, missed (too-late) resends/min, current profile.
    None
}

/// Specs for [`suggest_profile`], which is still `TODO(user)`. Against its placeholder `None`
/// they cannot fail, so they are ignored until the thresholds exist; then run them with
/// `cargo test -- --ignored advice` and remove the `#[ignore]`s. They encode choices that are
/// yours to make (a burst in the first seconds suggests nothing, light loss that is all resent
/// suggests nothing): change any that disagree with the thresholds you pick.
#[cfg(test)]
mod tests {
    use super::*;

    fn loss(resend_requests_per_min: f64, missed_per_min: f64) -> LossStats {
        LossStats {
            window: LOSS_WINDOW,
            resend_requests_per_min,
            missed_per_min,
        }
    }

    #[test]
    #[ignore = "spec for suggest_profile: enable after writing its thresholds"]
    fn no_loss_keeps_current_profile() {
        for p in LatencyProfile::ALL {
            assert_eq!(suggest_profile(p, &loss(0.0, 0.0)), None);
        }
    }

    #[test]
    #[ignore = "spec for suggest_profile: enable after writing its thresholds"]
    fn a_burst_in_the_first_seconds_suggests_nothing() {
        // Heavy-loss rates, but from 10 resends and 2 lost packets in the first 5 s, as a
        // receiver settles in: too little to judge the network by.
        let burst = LossStats {
            window: Duration::from_secs(5),
            resend_requests_per_min: 120.0,
            missed_per_min: 24.0,
        };
        assert_eq!(suggest_profile(LatencyProfile::Balanced, &burst), None);
    }

    #[test]
    #[ignore = "spec for suggest_profile: enable after writing its thresholds"]
    fn light_loss_that_is_all_resent_suggests_nothing() {
        assert_eq!(
            suggest_profile(LatencyProfile::Balanced, &loss(1.0, 0.0)),
            None
        );
    }

    #[test]
    #[ignore = "spec for suggest_profile: enable after writing its thresholds"]
    fn stable_has_nothing_more_robust_to_suggest() {
        assert_eq!(
            suggest_profile(LatencyProfile::Stable, &loss(600.0, 120.0)),
            None
        );
    }

    #[test]
    #[ignore = "spec for suggest_profile: enable after writing its thresholds"]
    fn never_suggests_a_less_robust_profile() {
        // Whatever the thresholds become, a suggestion must be more robust than the current one.
        let order = |p: LatencyProfile| LatencyProfile::ALL.iter().position(|q| *q == p).unwrap();
        for p in LatencyProfile::ALL {
            for (r, m) in [
                (0.0, 0.0),
                (5.0, 0.0),
                (30.0, 2.0),
                (120.0, 20.0),
                (300.0, 60.0),
            ] {
                if let Some(s) = suggest_profile(p, &loss(r, m)) {
                    assert!(order(s) > order(p), "{p:?} -> {s:?} for {r}/{m} per min");
                }
            }
        }
    }

    /// Expected behaviour once thresholds exist: heavy loss with unrecoverable packets on
    /// `balanced` (a receiver on lossy Wi-Fi) suggests `stable`; on `low` it suggests at least
    /// `balanced`.
    #[test]
    #[ignore = "spec for suggest_profile: enable after writing its thresholds"]
    fn heavy_loss_suggests_more_robust_profile() {
        let heavy = loss(120.0, 20.0);
        assert_eq!(
            suggest_profile(LatencyProfile::Balanced, &heavy),
            Some(LatencyProfile::Stable)
        );
        assert!(matches!(
            suggest_profile(LatencyProfile::Low, &heavy),
            Some(LatencyProfile::Balanced | LatencyProfile::Stable)
        ));
    }
}
