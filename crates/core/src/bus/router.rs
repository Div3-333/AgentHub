//! Bus routing task (blueprint §7.2). Pure routing lives in [`super::routing`];
//! LLM Racing lives in [`super::racing`].

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc};
use tracing::warn;
use uuid::Uuid;

use crate::bus::event::{BusEvent, MessageTarget, WorkspaceModeRepr};
use crate::bus::racing::{try_dispatch_racing_user_message, RacingDispatch, RacingRegistry};
use crate::bus::routing::{
    format_injection, resolve_mention_target, resolve_recipients, should_stagger, stagger_delay_ms,
    AgentRouteInfo, RouteAgentStatus, USER_SENDER_TAG,
};
use crate::bus::BUS_CHANNEL_CAPACITY;
use crate::db::DbClient;
use crate::pty::PtyStatus;
use crate::server::modes::{self, filter_recipients_by_channel};
use crate::server::ServerState;

fn route_status_from_pty(status: PtyStatus) -> RouteAgentStatus {
    match status {
        PtyStatus::Initializing => RouteAgentStatus::Initializing,
        PtyStatus::Idle => RouteAgentStatus::Idle,
        PtyStatus::Thinking => RouteAgentStatus::Thinking,
        PtyStatus::Muted => RouteAgentStatus::Muted,
        PtyStatus::Deafened => RouteAgentStatus::Deafened,
        PtyStatus::Suspended => RouteAgentStatus::Suspended,
        PtyStatus::Dead => RouteAgentStatus::Dead,
        PtyStatus::RateLimited => RouteAgentStatus::RateLimited,
    }
}

/// Channels created when the central bus router task is started.
pub struct BusRouterChannels {
    pub bus_tx: broadcast::Sender<BusEvent>,
    pub tui_rx: mpsc::UnboundedReceiver<BusEvent>,
    pub racing: Arc<RacingRegistry>,
}

/// Creates the broadcast bus pair (blueprint §7.2).
#[must_use]
pub fn create_bus_channel() -> (broadcast::Sender<BusEvent>, broadcast::Receiver<BusEvent>) {
    broadcast::channel(BUS_CHANNEL_CAPACITY)
}

/// Start the `BusRouter` task (blueprint §7.2 + §11 racing).
pub fn spawn_bus_router(
    state: Arc<ServerState>,
    db: Option<Arc<DbClient>>,
    session_id: Uuid,
) -> BusRouterChannels {
    let (bus_tx, _) = broadcast::channel(BUS_CHANNEL_CAPACITY);
    let (tui_tx, tui_rx) = mpsc::unbounded_channel();
    let mut bus_rx = bus_tx.subscribe();
    let racing = Arc::new(RacingRegistry::new());
    let racing_task = Arc::clone(&racing);
    let bus_tx_loop = bus_tx.clone();

    tokio::spawn(async move {
        loop {
            match bus_rx.recv().await {
                Ok(event) => {
                    let event = racing_task.process_bus_event(event, &bus_tx_loop);

                    if let Some(ref db) = db {
                        if let Err(e) = db.log_bus_event(session_id, &event).await {
                            warn!("failed to log bus event: {e}");
                        }
                        if let Err(e) = log_racing_side_effects(db, session_id, &event).await {
                            warn!("failed to log racing event: {e}");
                        }
                    }

                    match &event {
                        BusEvent::RacingComplete { session_id, .. }
                        | BusEvent::RacingCancelled { session_id, .. } => {
                            racing_task.finish(*session_id);
                        }
                        _ => {}
                    }

                    // Route to PTY before TUI forwarding so a slow TUI consumer cannot
                    // block stdin injection (integration smoke / headless runners).
                    let mode = WorkspaceModeRepr::from_atomic(state.mode.load(Ordering::Relaxed));
                    route_message_injection(
                        &state,
                        mode,
                        &event,
                        session_id,
                        db.as_deref(),
                        &racing_task,
                        &bus_tx_loop,
                    )
                    .await;

                    if tui_tx.send(event.clone()).is_err() {
                        warn!("TUI bus consumer disconnected");
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    warn!(skipped, "bus router lagged behind producers");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    BusRouterChannels {
        bus_tx,
        tui_rx,
        racing,
    }
}

async fn log_racing_side_effects(
    db: &DbClient,
    _hub_session_id: Uuid,
    event: &BusEvent,
) -> crate::error::Result<()> {
    if let BusEvent::RacingComplete { session_id, .. } = event {
        db.complete_race(*session_id).await?;
    }
    Ok(())
}

fn collect_route_info(state: &ServerState) -> Vec<AgentRouteInfo> {
    state
        .agents
        .iter()
        .map(|entry| {
            let agent = entry.value();
            AgentRouteInfo {
                id: agent.id,
                tag: agent.tag.clone(),
                status: agent
                    .status()
                    .map(route_status_from_pty)
                    .unwrap_or(RouteAgentStatus::Dead),
                receives_broadcast: agent.receives_broadcast.load(Ordering::Acquire),
            }
        })
        .collect()
}

fn count_thinking_agents(state: &ServerState) -> usize {
    state
        .agents
        .iter()
        .filter(|entry| entry.value().status() == Some(PtyStatus::Thinking))
        .count()
}

async fn inject_to_recipients(
    state: &ServerState,
    recipients: &[Uuid],
    payload: &str,
    mode: WorkspaceModeRepr,
    thinking_count: usize,
) {
    if recipients.is_empty() {
        return;
    }

    let stagger = should_stagger(mode, thinking_count, recipients.len());
    for (index, id) in recipients.iter().enumerate() {
        if stagger {
            let delay = Duration::from_millis(stagger_delay_ms(index));
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
        }
        if let Some(agent) = state.agents.get(id) {
            let prev = agent
                .status
                .swap(PtyStatus::Thinking.as_u8(), Ordering::AcqRel);
            if let Err(e) = agent.write_stdin(payload.as_bytes()) {
                warn!(agent = %agent.tag, "bus injection failed: {e}");
                agent.status.store(prev, Ordering::Release);
            }
        }
    }
}

async fn route_message_injection(
    state: &Arc<ServerState>,
    mode: WorkspaceModeRepr,
    event: &BusEvent,
    hub_session_id: Uuid,
    db: Option<&DbClient>,
    racing: &RacingRegistry,
    bus_tx: &broadcast::Sender<BusEvent>,
) {
    let agents = collect_route_info(state);
    let thinking_count = count_thinking_agents(state);

    match event {
        BusEvent::UserMessage {
            content, target, ..
        } => {
            match try_dispatch_racing_user_message(
                hub_session_id,
                Arc::clone(state),
                racing,
                db,
                content,
                None,
                bus_tx,
            )
            .await
            {
                RacingDispatch::Started | RacingDispatch::Failed => return,
                RacingDispatch::NotRacing => {}
            }

            let effective = resolve_mention_target(content, target, &agents);
            let mut recipients = resolve_recipients(&agents, mode, None, &effective, true);
            recipients = filter_recipients_by_channel(state, content, recipients);
            let payload = format_injection(USER_SENDER_TAG, content);
            inject_to_recipients(state, &recipients, &payload, mode, thinking_count).await;
        }
        BusEvent::AgentMessage {
            race_session_id: Some(_),
            ..
        } => {}
        BusEvent::AgentMessage {
            id, tag, content, ..
        } => {
            let target = if modes::agent_may_broadcast(state, *id) {
                MessageTarget::Broadcast
            } else {
                resolve_mention_target(content, &MessageTarget::Broadcast, &agents)
            };
            let mut recipients = resolve_recipients(&agents, mode, Some(*id), &target, false);
            recipients = filter_recipients_by_channel(state, content, recipients);
            let payload = format_injection(tag, content);
            inject_to_recipients(state, &recipients, &payload, mode, thinking_count).await;
        }
        _ => {}
    }
}

#[cfg(all(test, feature = "full"))]
mod tests {
    use super::*;
    use chrono::Utc;

    use crate::bus::event::MODE_GROUP_CHAT;
    use crate::pty::mock_agent_with_capture;

    fn group_chat_state() -> Arc<ServerState> {
        let state = Arc::new(ServerState::new());
        state.mode.store(MODE_GROUP_CHAT, Ordering::Relaxed);
        state
    }

    #[tokio::test]
    async fn user_message_injects_to_mock_agents() {
        let state = group_chat_state();
        let (a1, cap1) = mock_agent_with_capture(Uuid::new_v4(), "gemini-1", PtyStatus::Idle, true);
        let (a2, cap2) = mock_agent_with_capture(Uuid::new_v4(), "claude-2", PtyStatus::Idle, true);
        state.agents.insert(a1.id, a1);
        state.agents.insert(a2.id, a2);

        let channels = spawn_bus_router(Arc::clone(&state), None, Uuid::new_v4());
        let _ = channels.bus_tx.send(BusEvent::UserMessage {
            content: "hello all".into(),
            timestamp: Utc::now(),
            target: MessageTarget::Broadcast,
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let expected = format_injection(USER_SENDER_TAG, "hello all");
        assert_eq!(
            String::from_utf8_lossy(&cap1.lock().expect("cap1")),
            expected
        );
        assert_eq!(
            String::from_utf8_lossy(&cap2.lock().expect("cap2")),
            expected
        );
    }

    #[tokio::test]
    async fn user_broadcast_skips_deafened_agents() {
        let state = group_chat_state();
        let (listening, listening_cap) =
            mock_agent_with_capture(Uuid::new_v4(), "gemini-1", PtyStatus::Idle, true);
        let (deafened, deafened_cap) =
            mock_agent_with_capture(Uuid::new_v4(), "claude-2", PtyStatus::Idle, false);
        state.agents.insert(listening.id, listening);
        state.agents.insert(deafened.id, deafened);

        let channels = spawn_bus_router(Arc::clone(&state), None, Uuid::new_v4());
        let _ = channels.bus_tx.send(BusEvent::UserMessage {
            content: "broadcast check".into(),
            timestamp: Utc::now(),
            target: MessageTarget::Broadcast,
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let expected = format_injection(USER_SENDER_TAG, "broadcast check");
        assert_eq!(
            String::from_utf8_lossy(&listening_cap.lock().expect("listening cap")),
            expected
        );
        assert!(deafened_cap.lock().expect("deafened cap").is_empty());
    }

    #[tokio::test]
    async fn user_mention_routes_to_target_even_when_deafened() {
        let state = group_chat_state();
        let (mentioned, mentioned_cap) =
            mock_agent_with_capture(Uuid::new_v4(), "gemini-1", PtyStatus::Idle, false);
        let (other, other_cap) =
            mock_agent_with_capture(Uuid::new_v4(), "claude-2", PtyStatus::Idle, true);
        state.agents.insert(mentioned.id, mentioned);
        state.agents.insert(other.id, other);

        let channels = spawn_bus_router(Arc::clone(&state), None, Uuid::new_v4());
        let _ = channels.bus_tx.send(BusEvent::UserMessage {
            content: "@gemini-1 direct ping".into(),
            timestamp: Utc::now(),
            target: MessageTarget::Broadcast,
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let expected = format_injection(USER_SENDER_TAG, "@gemini-1 direct ping");
        assert_eq!(
            String::from_utf8_lossy(&mentioned_cap.lock().expect("mentioned cap")),
            expected
        );
        assert!(other_cap.lock().expect("other cap").is_empty());
    }

    #[tokio::test]
    async fn agent_message_excludes_sender() {
        let state = group_chat_state();
        let (gemini, gemini_cap) =
            mock_agent_with_capture(Uuid::new_v4(), "gemini-1", PtyStatus::Idle, true);
        let claude_id = Uuid::new_v4();
        let (claude, claude_cap) =
            mock_agent_with_capture(claude_id, "claude-2", PtyStatus::Idle, true);
        state.agents.insert(gemini.id, gemini);
        state.agents.insert(claude_id, claude);

        let channels = spawn_bus_router(Arc::clone(&state), None, Uuid::new_v4());
        let _ = channels.bus_tx.send(BusEvent::AgentMessage {
            id: claude_id,
            tag: "claude-2".into(),
            content: "team update".into(),
            timestamp: Utc::now(),
            race_session_id: None,
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        assert!(claude_cap.lock().expect("claude cap").is_empty());
        let expected = format_injection("claude-2", "team update");
        assert_eq!(
            String::from_utf8_lossy(&gemini_cap.lock().expect("gemini cap")),
            expected
        );
    }

    #[tokio::test]
    async fn racing_input_fans_out_raw_prompt_not_broadcast_injection() {
        let state = group_chat_state();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let (a1, cap1) = mock_agent_with_capture(id1, "mock-1", PtyStatus::Idle, true);
        let (a2, cap2) = mock_agent_with_capture(id2, "mock-2", PtyStatus::Idle, true);
        state.agents.insert(a1.id, a1);
        state.agents.insert(a2.id, a2);

        let channels = spawn_bus_router(Arc::clone(&state), None, Uuid::new_v4());
        let _ = channels.bus_tx.send(BusEvent::UserMessage {
            content: "@mock-1 @mock-2 write tests".into(),
            timestamp: Utc::now(),
            target: MessageTarget::Broadcast,
        });

        tokio::time::sleep(Duration::from_millis(80)).await;

        assert_eq!(
            String::from_utf8_lossy(&cap1.lock().expect("cap1")),
            "write tests\n"
        );
        assert_eq!(
            String::from_utf8_lossy(&cap2.lock().expect("cap2")),
            "write tests\n"
        );
        assert!(channels.racing.race_id_for_agent(id1).is_some());
        assert!(channels.racing.race_id_for_agent(id2).is_some());
    }

    #[tokio::test]
    async fn bus_router_forwards_events_to_tui_channel() {
        let state = group_chat_state();
        let channels = spawn_bus_router(state, None, Uuid::new_v4());
        let mut tui_rx = channels.tui_rx;

        let _ = channels.bus_tx.send(BusEvent::SystemMessage {
            content: "router→tui".into(),
            timestamp: Utc::now(),
        });

        let event = tokio::time::timeout(Duration::from_millis(200), tui_rx.recv())
            .await
            .expect("timeout")
            .expect("tui channel closed");
        assert!(matches!(
            event,
            BusEvent::SystemMessage { content, .. } if content == "router→tui"
        ));
    }

    #[tokio::test]
    async fn suspended_agent_receives_no_broadcast_injection() {
        let state = group_chat_state();
        let (active, active_cap) =
            mock_agent_with_capture(Uuid::new_v4(), "gemini-1", PtyStatus::Idle, true);
        let (suspended, suspended_cap) =
            mock_agent_with_capture(Uuid::new_v4(), "claude-2", PtyStatus::Suspended, true);
        state.agents.insert(active.id, active);
        state.agents.insert(suspended.id, suspended);

        let channels = spawn_bus_router(Arc::clone(&state), None, Uuid::new_v4());
        let _ = channels.bus_tx.send(BusEvent::UserMessage {
            content: "room broadcast".into(),
            timestamp: Utc::now(),
            target: MessageTarget::Broadcast,
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let expected = format_injection(USER_SENDER_TAG, "room broadcast");
        assert_eq!(
            String::from_utf8_lossy(&active_cap.lock().expect("active cap")),
            expected
        );
        assert!(suspended_cap.lock().expect("suspended cap").is_empty());
    }

    #[tokio::test]
    async fn stagger_delays_second_recipient_when_multiple_thinking() {
        let state = group_chat_state();
        let id_a = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let id_b = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let (a, cap_a) = mock_agent_with_capture(id_a, "agent-a", PtyStatus::Thinking, true);
        let (b, cap_b) = mock_agent_with_capture(id_b, "agent-b", PtyStatus::Thinking, true);
        state.agents.insert(a.id, a);
        state.agents.insert(b.id, b);

        let channels = spawn_bus_router(Arc::clone(&state), None, Uuid::new_v4());
        let _ = channels.bus_tx.send(BusEvent::UserMessage {
            content: "stagger probe".into(),
            timestamp: Utc::now(),
            target: MessageTarget::Broadcast,
        });

        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(!cap_a.lock().expect("cap_a").is_empty());
        assert!(cap_b.lock().expect("cap_b").is_empty());

        tokio::time::sleep(Duration::from_millis(120)).await;
        assert!(!cap_b.lock().expect("cap_b").is_empty());
    }
}
