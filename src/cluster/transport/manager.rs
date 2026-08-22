use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread;
use std::time::Duration;

use super::health::PeerHealth;
use super::{
    Frame, FrameCodec, MessageType, PeerConnection, PeerHealthSnapshot, RequestError,
    RequestRegistry, decode_topology,
};
use crate::cluster::server::stable_id;
use crate::cluster::{ClusterConfig, ClusterState, NodeInfo, decode_failure_report};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const MIN_BACKOFF: Duration = Duration::from_millis(100);
const MAX_BACKOFF: Duration = Duration::from_secs(5);

/// Owns exactly one bounded outbound queue and connection worker per peer.
/// Dropping the manager closes the queues and lets the workers exit.
pub struct PeerManager {
    peers: HashMap<String, PeerHandle>,
    next_request_id: AtomicU64,
    _workers: Vec<thread::JoinHandle<()>>,
    state: ClusterState,
}

struct PeerHandle {
    sender: SyncSender<Frame>,
    requests: RequestRegistry,
    health: PeerHealth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerSendError {
    UnknownPeer,
    QueueFull,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplicationSendError {
    UnknownPeer,
    QueueFull,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerRequestError {
    UnknownPeer,
    QueueFull,
    Disconnected,
    Registry(RequestError),
}

impl PeerManager {
    pub fn try_send(&self, peer_id: &str, frame: Frame) -> Result<(), PeerSendError> {
        let sender = &self
            .peers
            .get(peer_id)
            .ok_or(PeerSendError::UnknownPeer)?
            .sender;
        sender.try_send(frame).map_err(|error| match error {
            TrySendError::Full(_) => PeerSendError::QueueFull,
            TrySendError::Disconnected(_) => PeerSendError::Disconnected,
        })
    }

    pub fn try_send_replication(
        &self,
        peer_id: &str,
        frame: Frame,
    ) -> Result<(), ReplicationSendError> {
        self.try_send(peer_id, frame).map_err(|error| match error {
            PeerSendError::UnknownPeer => ReplicationSendError::UnknownPeer,
            PeerSendError::QueueFull => ReplicationSendError::QueueFull,
            PeerSendError::Disconnected => ReplicationSendError::Disconnected,
        })
    }

    pub fn request(
        &self,
        peer_id: &str,
        mut frame: Frame,
        timeout: Duration,
    ) -> Result<Frame, PeerRequestError> {
        let peer = self
            .peers
            .get(peer_id)
            .ok_or(PeerRequestError::UnknownPeer)?;
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        frame.request_id = request_id;
        peer.requests
            .register(request_id)
            .map_err(PeerRequestError::Registry)?;
        if let Err(error) = peer.sender.try_send(frame) {
            let _ = peer.requests.cancel(request_id);
            return Err(match error {
                TrySendError::Full(_) => PeerRequestError::QueueFull,
                TrySendError::Disconnected(_) => PeerRequestError::Disconnected,
            });
        }
        peer.requests
            .wait(request_id, timeout)
            .map_err(PeerRequestError::Registry)
    }

    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    pub fn health(&self, peer_id: &str) -> Option<PeerHealthSnapshot> {
        self.peers.get(peer_id).map(|peer| peer.health.snapshot())
    }

    pub fn record_failure_report(&self, payload: &[u8]) -> bool {
        decode_failure_report(payload).is_some_and(|report| self.state.record_failure(report))
    }

    pub fn suspect_peers(&self, timeout: Duration) -> Vec<String> {
        self.peers
            .iter()
            .filter(|(_, peer)| peer.health.is_suspect(timeout))
            .map(|(id, _)| id.clone())
            .collect()
    }
}

pub fn start_peer_manager(config: &ClusterConfig, state: ClusterState) -> PeerManager {
    let mut peers = HashMap::new();
    let mut workers = Vec::new();
    for node in config
        .topology
        .nodes
        .iter()
        .filter(|node| node.id != config.local_id)
    {
        let (sender, receiver) = mpsc::sync_channel(config.peer_queue_capacity);
        let requests = RequestRegistry::new(config.peer_queue_capacity);
        let health = PeerHealth::new();
        peers.insert(
            node.id.clone(),
            PeerHandle {
                sender,
                requests: requests.clone(),
                health: health.clone(),
            },
        );
        let local_id = config.local_id.clone();
        let local_epoch = config.topology.epoch;
        let heartbeat_interval = config.heartbeat_interval;
        let auth_token = config.auth_token.clone();
        let node = node.clone();
        if let Ok(worker) = thread::Builder::new()
            .name(format!("fyrodb-cluster-out-{}", node.id))
            .stack_size(64 * 1024)
            .spawn(move || {
                run_peer_worker(
                    receiver,
                    requests,
                    health,
                    &local_id,
                    local_epoch,
                    &node,
                    heartbeat_interval,
                    auth_token.as_deref(),
                )
            })
        {
            workers.push(worker);
        }
    }
    PeerManager {
        peers,
        next_request_id: AtomicU64::new(1),
        _workers: workers,
        state,
    }
}

#[allow(clippy::too_many_arguments)]
fn run_peer_worker(
    receiver: mpsc::Receiver<Frame>,
    requests: RequestRegistry,
    health: PeerHealth,
    local_id: &str,
    epoch: u64,
    node: &NodeInfo,
    heartbeat_interval: Duration,
    auth_token: Option<&str>,
) {
    let Ok(address) = node.cluster_address.parse::<SocketAddr>() else {
        return;
    };
    let mut peer = None;
    let mut backoff = MIN_BACKOFF;
    let mut request_id = 1u64;
    loop {
        let frame = match receiver.recv_timeout(heartbeat_interval) {
            Ok(frame) => frame,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let frame = Frame {
                    message_type: MessageType::Ping,
                    flags: 0,
                    request_id,
                    source_id: stable_id(local_id),
                    target_id: stable_id(&node.id),
                    epoch,
                    payload: Vec::new(),
                };
                request_id = request_id.wrapping_add(1).max(1);
                frame
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        loop {
            if peer.is_none() {
                health.connecting();
                match connect_and_handshake(address, local_id, node, epoch, auth_token) {
                    Ok(connection) => {
                        let generation = health.connected();
                        if let Ok(reader) = connection.try_clone() {
                            let pending = requests.clone();
                            let peer_health = health.clone();
                            thread::Builder::new()
                                .name(format!("fyrodb-cluster-in-{}", node.id))
                                .stack_size(64 * 1024)
                                .spawn(move || {
                                    read_replies(reader, pending, peer_health, generation)
                                })
                                .ok();
                        }
                        peer = Some(connection);
                        backoff = MIN_BACKOFF;
                    }
                    Err(_) => {
                        health.disconnected(None);
                        thread::sleep(backoff);
                        backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
                        continue;
                    }
                }
            }
            if peer
                .as_mut()
                .is_some_and(|connection| connection.send(&frame).is_ok())
            {
                break;
            }
            peer = None;
            health.disconnected(None);
            if backoff < MAX_BACKOFF {
                thread::sleep(backoff);
                backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
            }
        }
    }
}

fn read_replies(
    mut peer: PeerConnection,
    requests: RequestRegistry,
    health: PeerHealth,
    generation: u64,
) {
    while let Ok(frame) = peer.receive() {
        if frame.message_type == MessageType::Pong {
            health.pong(generation);
        }
        if matches!(
            frame.message_type,
            MessageType::CommandReply | MessageType::Pong
        ) {
            let _ = requests.complete(frame);
        }
    }
    health.disconnected(Some(generation));
    requests.fail_pending();
}

fn connect_and_handshake(
    address: SocketAddr,
    local_id: &str,
    node: &NodeInfo,
    epoch: u64,
    auth_token: Option<&str>,
) -> Result<PeerConnection, super::ProtocolError> {
    let mut peer = PeerConnection::connect(address, CONNECT_TIMEOUT, FrameCodec::default())?;
    peer.send(&Frame {
        message_type: MessageType::Hello,
        flags: 0,
        request_id: 0,
        source_id: stable_id(local_id),
        target_id: stable_id(&node.id),
        epoch,
        payload: match auth_token {
            Some(token) => format!("{token}\0{local_id}").into_bytes(),
            None => local_id.as_bytes().to_vec(),
        },
    })?;
    let response = peer.receive()?;
    if response.message_type != MessageType::Hello || response.payload != node.id.as_bytes() {
        return Err(super::ProtocolError::InvalidHandshake);
    }
    let topology = peer.receive()?;
    if topology.message_type != MessageType::Topology || topology.epoch < epoch {
        return Err(super::ProtocolError::InvalidHandshake);
    }
    decode_topology(&topology.payload).map_err(|_| super::ProtocolError::InvalidHandshake)?;
    peer.set_read_timeout(None)?;
    Ok(peer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::{NodeRole, Slot, SlotRange, Topology};

    fn config() -> ClusterConfig {
        let node = |id: &str, port| NodeInfo {
            id: id.into(),
            address: format!("127.0.0.1:{port}"),
            cluster_address: format!("127.0.0.1:{}", port + 10_000),
            role: NodeRole::Primary,
            epoch: 1,
            slots: vec![SlotRange::new(Slot(0), Slot(1)).unwrap()],
        };
        ClusterConfig {
            enabled: true,
            local_id: "a".into(),
            listen_address: "127.0.0.1:18000".into(),
            peer_queue_capacity: 16,
            heartbeat_interval: Duration::from_secs(2),
            suspect_timeout: Duration::from_secs(6),
            failure_quorum: 2,
            max_inbound_peers: 16,
            auth_token: None,
            replication_log_capacity: 16,
            topology: Topology::new(1, vec![node("a", 8000), node("b", 8001)]),
        }
    }

    #[test]
    fn creates_only_remote_peer_queues() {
        let manager = start_peer_manager(&config(), ClusterState::new(2, Duration::from_secs(30)));
        assert_eq!(manager.peer_count(), 1);
        let frame = Frame {
            message_type: MessageType::Ping,
            flags: 0,
            request_id: 1,
            source_id: 1,
            target_id: 2,
            epoch: 1,
            payload: Vec::new(),
        };
        assert_eq!(
            manager.try_send("missing", frame),
            Err(PeerSendError::UnknownPeer)
        );
    }
}
