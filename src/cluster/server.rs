use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use super::{
    ClusterConfig, ClusterState, Frame, FrameCodec, MessageType, PeerConnection, ReplicationStream,
    decode_failure_report, decode_replication_message, encode_topology,
};

struct PeerPermit(Arc<AtomicUsize>);
impl Drop for PeerPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Start the dedicated cluster listener. The listener is intentionally
/// separate from client workers so malformed or slow peer traffic cannot
/// consume client connection slots.
pub fn start_listener(
    config: ClusterConfig,
    state: ClusterState,
) -> io::Result<thread::JoinHandle<()>> {
    let Some(_) = config.local_node() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "cluster node is not in topology",
        ));
    };
    let address: SocketAddr = config.listen_address.parse().map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid cluster address: {error}"),
        )
    })?;
    let listener = TcpListener::bind(address)?;
    listener.set_nonblocking(false)?;
    let local_id = config.local_id.clone();
    let epoch = config.topology.epoch;
    let topology = config.topology.clone();
    let codec = FrameCodec::default();
    let active_peers = Arc::new(AtomicUsize::new(0));
    let max_inbound_peers = config.max_inbound_peers;
    let peer_timeout = config.suspect_timeout;
    let auth_token = config.auth_token.clone();
    let handle = thread::Builder::new()
        .name("fyrodb-cluster-listener".into())
        .stack_size(64 * 1024)
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        if active_peers.fetch_add(1, Ordering::Relaxed) >= max_inbound_peers {
                            active_peers.fetch_sub(1, Ordering::Relaxed);
                            continue;
                        }
                        let permit = PeerPermit(Arc::clone(&active_peers));
                        let peer_id = local_id.clone();
                        let peer_codec = codec.clone();
                        let peer_topology = topology.clone();
                        let peer_state = state.clone();
                        let peer_auth = auth_token.clone();
                        let peer_timeout = peer_timeout;
                        let _ = thread::Builder::new()
                            .name("fyrodb-cluster-peer".into())
                            .stack_size(64 * 1024)
                            .spawn(move || {
                                let _permit = permit;
                                handle_peer(
                                    stream,
                                    peer_codec,
                                    &peer_id,
                                    epoch,
                                    &peer_topology,
                                    &peer_state,
                                    peer_auth.as_deref(),
                                    peer_timeout,
                                )
                            });
                    }
                    Err(error) => eprintln!("fyrodb cluster accept error: {error}"),
                }
            }
        })?;
    Ok(handle)
}

#[allow(clippy::too_many_arguments)]
fn handle_peer(
    stream: TcpStream,
    codec: FrameCodec,
    local_id: &str,
    epoch: u64,
    topology: &super::Topology,
    state: &ClusterState,
    auth_token: Option<&str>,
    peer_timeout: std::time::Duration,
) {
    let Ok(mut peer) = PeerConnection::from_stream(stream, codec) else {
        return;
    };
    if peer.set_read_timeout(Some(peer_timeout)).is_err()
        || peer.set_write_timeout(Some(peer_timeout)).is_err()
    {
        return;
    }
    let Ok(hello) = peer.receive() else { return };
    if hello.message_type != MessageType::Hello || hello.payload.is_empty() {
        return;
    }
    let Some((remote_id, received_token)) = parse_hello(&hello.payload) else {
        return;
    };
    if !auth_matches(auth_token, received_token) {
        return;
    }
    if remote_id == local_id {
        return;
    }
    let response = Frame {
        message_type: MessageType::Hello,
        flags: 0,
        request_id: hello.request_id,
        source_id: stable_id(local_id),
        target_id: hello.source_id,
        epoch,
        payload: local_id.as_bytes().to_vec(),
    };
    if peer.send(&response).is_err() {
        return;
    }
    let Ok(payload) = encode_topology(topology) else {
        return;
    };
    if peer
        .send(&Frame {
            message_type: MessageType::Topology,
            flags: 0,
            request_id: hello.request_id,
            source_id: stable_id(local_id),
            target_id: hello.source_id,
            epoch,
            payload,
        })
        .is_err()
    {
        return;
    }
    let mut replication = ReplicationStream::default();
    while let Ok(frame) = peer.receive() {
        match frame.message_type {
            MessageType::Ping => {
                let _ = peer.send(&Frame {
                    message_type: MessageType::Pong,
                    flags: 0,
                    request_id: frame.request_id,
                    source_id: stable_id(local_id),
                    target_id: frame.source_id,
                    epoch,
                    payload: Vec::new(),
                });
            }
            MessageType::Hello => {}
            MessageType::FailureReport => {
                if let Some(report) = decode_failure_report(&frame.payload) {
                    state.record_failure(report);
                }
            }
            MessageType::ReplicationBegin
            | MessageType::ReplicationEntry
            | MessageType::ReplicationAck
            | MessageType::ReplicationSnapshot
            | MessageType::ReplicationFinish => {
                if let Ok(message) = decode_replication_message(&frame.payload) {
                    let _ = replication.apply(&message);
                }
            }
            _ => break,
        }
    }
}

pub(crate) fn stable_id(id: &str) -> u64 {
    id.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x1000_0000_01b3)
    })
}

fn parse_hello(payload: &[u8]) -> Option<(&str, Option<&str>)> {
    let value = std::str::from_utf8(payload).ok()?;
    value
        .split_once('\0')
        .map_or(Some((value, None)), |(token, id)| Some((id, Some(token))))
}

fn auth_matches(expected: Option<&str>, received: Option<&str>) -> bool {
    match (expected, received) {
        (None, _) => true,
        (Some(expected), Some(received)) => {
            let a = expected.as_bytes();
            let b = received.as_bytes();
            let mut diff = a.len() ^ b.len();
            for index in 0..a.len().max(b.len()) {
                diff |= usize::from(a.get(index) != b.get(index));
            }
            diff == 0
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    use super::{
        ClusterState, Frame, FrameCodec, MessageType, PeerConnection, auth_matches, handle_peer,
        stable_id,
    };

    #[test]
    fn stable_ids_are_deterministic() {
        assert_eq!(stable_id("node-a"), stable_id("node-a"));
        assert_ne!(stable_id("node-a"), stable_id("node-b"));
    }

    #[test]
    fn hello_auth_requires_matching_secret() {
        assert!(auth_matches(Some("secret"), Some("secret")));
        assert!(!auth_matches(Some("secret"), Some("wrong")));
        assert!(!auth_matches(Some("secret"), None));
        assert!(auth_matches(None, None));
    }

    #[test]
    #[ignore = "requires loopback socket permissions"]
    fn hello_handshake_and_ping_pong_work() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_peer(
                stream,
                FrameCodec::default(),
                "node-a",
                7,
                &super::super::Topology::default(),
                &ClusterState::new(2, std::time::Duration::from_secs(30)),
                None,
                std::time::Duration::from_secs(6),
            );
        });

        let stream = TcpStream::connect(address).unwrap();
        let mut peer = PeerConnection::from_stream(stream, FrameCodec::default()).unwrap();
        peer.send(&Frame {
            message_type: MessageType::Hello,
            flags: 0,
            request_id: 11,
            source_id: stable_id("node-b"),
            target_id: stable_id("node-a"),
            epoch: 1,
            payload: b"node-b".to_vec(),
        })
        .unwrap();
        let hello = peer.receive().unwrap();
        assert_eq!(hello.message_type, MessageType::Hello);
        assert_eq!(hello.request_id, 11);
        assert_eq!(hello.epoch, 7);
        assert_eq!(hello.payload, b"node-a");
        let topology = peer.receive().unwrap();
        assert_eq!(topology.message_type, MessageType::Topology);

        peer.send(&Frame {
            message_type: MessageType::Ping,
            flags: 0,
            request_id: 12,
            source_id: stable_id("node-b"),
            target_id: stable_id("node-a"),
            epoch: 1,
            payload: Vec::new(),
        })
        .unwrap();
        let pong = peer.receive().unwrap();
        assert_eq!(pong.message_type, MessageType::Pong);
        assert_eq!(pong.request_id, 12);
        drop(peer);
        server.join().unwrap();
    }
}
