//! The note in the footer when the rooms that play are on different latency profiles.
//!
//! Rooms aren't clock-synced to each other: each plays its profile's lag behind the PC, so two
//! rooms on different profiles play the difference apart. The profile in each row already
//! shows the lag behind the PC; this offset between rooms is the one thing the rows don't show,
//! so the note appears only when there is one.

use super::profile_title;
use crate::controller::ReceiverView;
use crate::session::Status;

/// The rooms that play now or, when stopped, will play on Play. While playing, a room that
/// failed does not count; one still idle does, since the capture opens before its session
/// starts (counting it avoids the note blinking off on Play).
fn active(r: &ReceiverView, streaming: bool) -> bool {
    r.selected && (!streaming || r.restarting || !matches!(r.status, Status::Error(_)))
}

/// "Kitchen and Bedroom TV play 92 ms apart. Put both on Stable to line them up.", or `None`
/// when all the active rooms share one profile. It suggests the slowest profile in use, the
/// one change that makes no room more likely to glitch.
pub(super) fn sync_note(receivers: &[ReceiverView], streaming: bool) -> Option<String> {
    let rooms: Vec<&ReceiverView> = receivers.iter().filter(|r| active(r, streaming)).collect();
    let fastest = rooms.iter().map(|r| r.profile).min_by_key(|p| p.lag_ms())?;
    let slowest = rooms.iter().map(|r| r.profile).max_by_key(|p| p.lag_ms())?;
    if fastest == slowest {
        return None;
    }
    let gap = slowest.lag_ms() - fastest.lag_ms();
    let (who, all) = match rooms.as_slice() {
        [a, b] => (
            format!("{} and {} play {gap} ms apart.", a.name, b.name),
            "both",
        ),
        _ => (
            format!("Rooms on different profiles play up to {gap} ms apart."),
            "them all",
        ),
    };
    Some(format!(
        "{who} Put {all} on {} to line them up.",
        profile_title(slowest)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::Reconnect;
    use crate::session::LatencyProfile::{self, *};

    fn room(name: &str, profile: LatencyProfile, selected: bool, status: Status) -> ReceiverView {
        ReceiverView {
            name: name.into(),
            addr: "192.0.2.10:7000".parse().unwrap(),
            model: None,
            online: true,
            selected,
            volume_pct: 50.0,
            volume_known: true,
            status,
            stats: None,
            profile,
            restarting: false,
            reconnect: Reconnect::No,
            suggestion: None,
            hidden: false,
        }
    }

    #[test]
    fn no_note_when_the_rooms_share_a_profile() {
        let rooms = [
            room("Kitchen", Balanced, true, Status::Idle),
            room("Bedroom TV", Balanced, true, Status::Idle),
            // Off, so its profile doesn't matter.
            room("Garage", Stable, false, Status::Idle),
        ];
        assert_eq!(sync_note(&rooms, false), None);
    }

    #[test]
    fn two_rooms_are_named_with_the_gap_and_the_slower_profile() {
        let rooms = [
            room("Kitchen", Balanced, true, Status::Idle),
            room("Bedroom TV", Stable, true, Status::Idle),
        ];
        assert_eq!(
            sync_note(&rooms, false).as_deref(),
            Some("Kitchen and Bedroom TV play 92 ms apart. Put both on Stable to line them up.")
        );
    }

    #[test]
    fn more_rooms_get_the_widest_gap() {
        let rooms = [
            room("Kitchen", Low, true, Status::Idle),
            room("Bedroom TV", Balanced, true, Status::Idle),
            room("Study", Stable, true, Status::Idle),
        ];
        assert_eq!(
            sync_note(&rooms, false).as_deref(),
            Some(
                "Rooms on different profiles play up to 132 ms apart. Put them all on Stable to line them up."
            )
        );
    }

    #[test]
    fn rooms_waiting_for_the_capture_to_open_count() {
        // Just after Play: playing, sessions not started yet.
        let rooms = [
            room("Kitchen", Balanced, true, Status::Idle),
            room("Bedroom TV", Stable, true, Status::Idle),
        ];
        assert!(sync_note(&rooms, true).is_some());
    }

    #[test]
    fn while_playing_a_failed_room_does_not_count() {
        let rooms = [
            room("Kitchen", Balanced, true, Status::Streaming),
            room("Bedroom TV", Stable, true, Status::Error("refused".into())),
        ];
        assert_eq!(sync_note(&rooms, true), None);
    }
}
