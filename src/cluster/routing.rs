use super::{ClusterConfig, Slot, hash_slot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteDecision<'a> {
    Local,
    Moved { slot: Slot, address: &'a str },
    CrossSlot,
    Unassigned(Slot),
}

pub fn route_command<'a>(
    cluster: &'a ClusterConfig,
    command: &[u8],
    args: &[&[u8]],
) -> RouteDecision<'a> {
    if !cluster.enabled {
        return RouteDecision::Local;
    }
    let Some(keys) = command_keys(command, args) else {
        return RouteDecision::Local;
    };
    let mut keys = keys.into_iter();
    let Some(first_key) = keys.next() else {
        return RouteDecision::Local;
    };
    let slot = hash_slot(first_key);
    if keys.any(|key| hash_slot(key) != slot) {
        return RouteDecision::CrossSlot;
    }
    match cluster.topology.owner(slot) {
        Some(owner) if owner.id == cluster.local_id => RouteDecision::Local,
        Some(owner) => RouteDecision::Moved {
            slot,
            address: &owner.address,
        },
        None => RouteDecision::Unassigned(slot),
    }
}

fn command_keys<'a>(command: &[u8], args: &'a [&'a [u8]]) -> Option<Vec<&'a [u8]>> {
    if command.eq_ignore_ascii_case(b"MGET")
        || command.eq_ignore_ascii_case(b"DEL")
        || command.eq_ignore_ascii_case(b"UNLINK")
        || command.eq_ignore_ascii_case(b"EXISTS")
        || command.eq_ignore_ascii_case(b"TOUCH")
    {
        return Some(args.to_vec());
    }
    if command.eq_ignore_ascii_case(b"MSET") || command.eq_ignore_ascii_case(b"MSETNX") {
        return Some(args.iter().step_by(2).copied().collect());
    }
    if command.eq_ignore_ascii_case(b"RENAME")
        || command.eq_ignore_ascii_case(b"RENAMENX")
        || command.eq_ignore_ascii_case(b"COPY")
        || command.eq_ignore_ascii_case(b"SMOVE")
        || command.eq_ignore_ascii_case(b"LMOVE")
        || command.eq_ignore_ascii_case(b"RPOPLPUSH")
    {
        return Some(args.iter().take(2).copied().collect());
    }
    if is_single_key(command) {
        return Some(args.first().copied().into_iter().collect());
    }
    None
}

fn is_single_key(command: &[u8]) -> bool {
    const COMMANDS: &[&[u8]] = &[
        b"GET",
        b"SET",
        b"SETNX",
        b"SETEX",
        b"PSETEX",
        b"GETDEL",
        b"GETSET",
        b"GETEX",
        b"INCR",
        b"DECR",
        b"INCRBY",
        b"DECRBY",
        b"INCRBYFLOAT",
        b"APPEND",
        b"STRLEN",
        b"GETRANGE",
        b"SETRANGE",
        b"TYPE",
        b"TTL",
        b"PTTL",
        b"EXPIRE",
        b"PEXPIRE",
        b"EXPIREAT",
        b"PERSIST",
        b"HSET",
        b"HGET",
        b"HMGET",
        b"HMSET",
        b"HGETALL",
        b"HDEL",
        b"HEXISTS",
        b"HLEN",
        b"HKEYS",
        b"HVALS",
        b"HINCRBY",
        b"HINCRBYFLOAT",
        b"LPUSH",
        b"RPUSH",
        b"LPOP",
        b"RPOP",
        b"LLEN",
        b"LINDEX",
        b"LSET",
        b"LRANGE",
        b"LTRIM",
        b"LREM",
        b"LINSERT",
        b"LPOS",
        b"SADD",
        b"SREM",
        b"SISMEMBER",
        b"SMISMEMBER",
        b"SMEMBERS",
        b"SCARD",
        b"SPOP",
        b"SRANDMEMBER",
        b"ZADD",
        b"ZREM",
        b"ZSCORE",
        b"ZMSCORE",
        b"ZRANK",
        b"ZREVRANK",
        b"ZCARD",
        b"ZCOUNT",
        b"ZINCRBY",
        b"ZRANGE",
        b"ZREVRANGE",
        b"ZRANGEBYSCORE",
        b"ZPOPMIN",
        b"ZPOPMAX",
        b"SETBIT",
        b"GETBIT",
        b"BITCOUNT",
        b"BITPOS",
        b"PFADD",
        b"PFCOUNT",
        b"JSON.SET",
        b"JSON.GET",
        b"JSON.DEL",
        b"JSON.TYPE",
        b"XADD",
        b"XLEN",
        b"XRANGE",
        b"XREVRANGE",
        b"XDEL",
        b"GEOADD",
        b"GEOPOS",
        b"GEODIST",
        b"GEOHASH",
        b"GEOSEARCH",
    ];
    COMMANDS
        .iter()
        .any(|known| command.eq_ignore_ascii_case(known))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::{NodeInfo, NodeRole, SlotRange, Topology};

    fn config() -> ClusterConfig {
        let local_slot = hash_slot(b"local");
        let remote_slot = hash_slot(b"remote");
        ClusterConfig {
            enabled: true,
            local_id: "a".into(),
            listen_address: "127.0.0.1:18000".into(),
            peer_queue_capacity: 16,
            heartbeat_interval: std::time::Duration::from_secs(2),
            suspect_timeout: std::time::Duration::from_secs(6),
            failure_quorum: 2,
            max_inbound_peers: 16,
            auth_token: None,
            replication_log_capacity: 16,
            topology: Topology::new(
                1,
                vec![
                    NodeInfo {
                        id: "a".into(),
                        address: "a:8000".into(),
                        cluster_address: "a:18000".into(),
                        role: NodeRole::Primary,
                        epoch: 1,
                        slots: vec![SlotRange::new(local_slot, local_slot).unwrap()],
                    },
                    NodeInfo {
                        id: "b".into(),
                        address: "b:8000".into(),
                        cluster_address: "b:18000".into(),
                        role: NodeRole::Primary,
                        epoch: 1,
                        slots: vec![SlotRange::new(remote_slot, remote_slot).unwrap()],
                    },
                ],
            ),
        }
    }

    #[test]
    fn routes_local_remote_and_cross_slot() {
        let config = config();
        assert_eq!(
            route_command(&config, b"GET", &[b"local"]),
            RouteDecision::Local
        );
        assert!(matches!(
            route_command(&config, b"GET", &[b"remote"]),
            RouteDecision::Moved { .. }
        ));
        assert_eq!(
            route_command(&config, b"MGET", &[b"local", b"remote"]),
            RouteDecision::CrossSlot
        );
        assert_eq!(
            route_command(&config, b"MGET", &[b"a{same}", b"b{same}"]),
            RouteDecision::Unassigned(hash_slot(b"{same}"))
        );
    }
}
