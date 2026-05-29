//! Central message bus (blueprint §7).

pub mod event;

#[cfg(any(feature = "full", feature = "bus-tests"))]
pub mod racing;
#[cfg(any(feature = "full", feature = "bus-tests"))]
pub mod router;
pub mod routing;

/// Broadcast channel capacity (blueprint §7.2).
pub const BUS_CHANNEL_CAPACITY: usize = 1024;
/// Backward-compatible alias for older callsites.
pub const BUS_CAPACITY: usize = BUS_CHANNEL_CAPACITY;

pub use event::{BusEvent, MessageTarget, OfflineReason, WorkspaceModeRepr};
pub use event::{MODE_DM, MODE_GROUP_CHAT, MODE_SERVER};

#[cfg(any(feature = "full", feature = "bus-tests"))]
pub use router::{create_bus_channel, spawn_bus_router, BusRouterChannels};

#[cfg(any(feature = "full", feature = "bus-tests"))]
pub use racing::{
    agent_tag_registry, inject_racing_prompts, inject_racing_prompts_on_state, is_racing_input,
    normalize_racing_tag, parse_racing_input, resolve_contestants_by_tag, start_racing_session,
    try_dispatch_racing_user_message, InjectRecord, ParsedRacingInput, RacingContestant,
    RacingDispatch, RacingRegistry, RacingSession, RacingSessionStart, INJECT_SPREAD_MS,
};

pub use routing::{
    format_injection, parse_mention_tags, resolve_mention_target, resolve_recipients,
    should_stagger, stagger_delay_ms, user_delivery_is_direct, AgentRouteInfo, RouteAgentStatus,
    STAGGER_STEP_MS, USER_SENDER_TAG,
};

#[cfg(test)]
mod tests {
    use super::BUS_CHANNEL_CAPACITY;

    #[test]
    fn bus_capacity_is_1024() {
        assert_eq!(BUS_CHANNEL_CAPACITY, 1024);
    }
}
