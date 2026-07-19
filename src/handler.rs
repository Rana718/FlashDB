use crate::commends;
use crate::pubsub::{encode_sub_reply, PubSub, SubSlot, WorkerNotifier};
use crate::storage::store::Store;
use crate::utils::parser::{ParseResult, RespParser};
use crate::utils::resp;
use mio::net::TcpStream;
use std::collections::HashSet;
use std::io::{self, Read, Write};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Connection mode
// ---------------------------------------------------------------------------

pub enum ConnMode {
    /// Normal key/value command mode.
    Normal,
    /// Pub/Sub mode — only the pub/sub command family is allowed.
    Subscribed {
        slot: Arc<SubSlot>,
        channels: HashSet<String>,
        patterns: HashSet<String>,
    },
}

// ---------------------------------------------------------------------------
// Conn
// ---------------------------------------------------------------------------

pub struct Conn {
    pub stream: TcpStream,
    pub parser: RespParser,
    pub store: Arc<Store>,
    pub pubsub: Arc<PubSub>,
    pub write_offset: usize,
    pub mode: ConnMode,
    pub token: usize,
    /// Worker notifier — shared with all SubSlots on this worker.
    pub notifier: Arc<WorkerNotifier>,
}

impl Conn {
    pub fn new(
        stream: TcpStream,
        store: Arc<Store>,
        pubsub: Arc<PubSub>,
        token: usize,
        notifier: Arc<WorkerNotifier>,
    ) -> Self {
        store.client_connected();
        Self {
            stream,
            parser: RespParser::new(),
            store,
            pubsub,
            write_offset: 0,
            mode: ConnMode::Normal,
            token,
            notifier,
        }
    }

    // -----------------------------------------------------------------------
    // Called by the event loop on readable events
    // -----------------------------------------------------------------------

    /// Returns `false` if the connection should be closed.
    pub fn do_read(&mut self) -> bool {
        loop {
            let buf = self.parser.read_buf();
            match self.stream.read(buf) {
                Ok(0) => return false,
                Ok(n) => self.parser.did_fill(n),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => return false,
            }
        }

        loop {
            match self.parser.parse_one() {
                ParseResult::Complete => {
                    // Zero-copy: convert raw pointer parts to &str on the stack.
                    let parts_raw = self.parser.parts_raw.as_slice();
                    const STACK_CAP: usize = 32;
                    if parts_raw.len() <= STACK_CAP {
                        let mut arr = [""; STACK_CAP];
                        for (i, &(ptr, len)) in parts_raw.iter().enumerate() {
                            arr[i] = unsafe {
                                std::str::from_utf8_unchecked(
                                    std::slice::from_raw_parts(ptr, len),
                                )
                            };
                        }
                        self.dispatch(&arr[..parts_raw.len()]);
                    } else {
                        let parts: Vec<&str> = parts_raw
                            .iter()
                            .map(|&(ptr, len)| unsafe {
                                std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len))
                            })
                            .collect();
                        self.dispatch(&parts);
                    }
                }
                ParseResult::Incomplete => break,
                ParseResult::Error => return false,
            }
        }

        true
    }

    // -----------------------------------------------------------------------
    // Called by the event loop on writable events or waker fire
    // -----------------------------------------------------------------------

    /// Drain pending pub/sub messages into wbuf, then flush wbuf to socket.
    /// Returns `false` if the connection should be closed.
    pub fn do_write(&mut self) -> bool {
        // Pull any pending pub/sub messages into the write buffer first.
        if let ConnMode::Subscribed { ref slot, .. } = self.mode {
            slot.drain_into(&mut self.parser.wbuf);
        }

        if self.parser.wbuf.is_empty() {
            return true;
        }

        loop {
            match self.stream.write(&self.parser.wbuf[self.write_offset..]) {
                Ok(n) => {
                    self.write_offset += n;
                    if self.write_offset >= self.parser.wbuf.len() {
                        self.write_offset = 0;
                        self.parser.wbuf.clear();
                        return true;
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return true,
                Err(_) => return false,
            }
        }
    }

    /// True when there is data waiting to be flushed (from commands or
    /// from pub/sub messages that arrived while the socket was idle).
    pub fn has_pending_write(&self) -> bool {
        !self.parser.wbuf.is_empty()
            || matches!(&self.mode, ConnMode::Subscribed { slot, .. } if slot.has_pending())
    }

    // -----------------------------------------------------------------------
    // Command dispatch
    // -----------------------------------------------------------------------

    fn dispatch(&mut self, parts: &[&str]) {
        if parts.is_empty() {
            self.parser.wbuf.extend_from_slice(b"-ERR empty command\r\n");
            return;
        }

        let cmd = parts[0].as_bytes();

        match &self.mode {
            ConnMode::Normal => {
                if eq_ignore_ascii(cmd, b"SUBSCRIBE") {
                    drop_mode_then(self, |c| c.handle_subscribe(parts));
                } else if eq_ignore_ascii(cmd, b"PSUBSCRIBE") {
                    drop_mode_then(self, |c| c.handle_psubscribe(parts));
                } else if eq_ignore_ascii(cmd, b"UNSUBSCRIBE") || eq_ignore_ascii(cmd, b"PUNSUBSCRIBE") {
                    self.parser.wbuf.extend_from_slice(&encode_sub_reply("unsubscribe", "", 0));
                } else if eq_ignore_ascii(cmd, b"PUBLISH") {
                    match parts {
                        [_, channel, message] => {
                            let n = self.pubsub.publish(channel, message);
                            resp::write_integer(&mut self.parser.wbuf, n as i64);
                        }
                        _ => resp::write_wrong_args(&mut self.parser.wbuf, "publish"),
                    }
                } else if eq_ignore_ascii(cmd, b"PUBSUB") {
                    pubsub_info(parts, &self.pubsub, &mut self.parser.wbuf);
                } else {
                    commends::execute(parts, &self.store, &mut self.parser.wbuf);
                }
            }

            ConnMode::Subscribed { .. } => {
                if eq_ignore_ascii(cmd, b"SUBSCRIBE") {
                    drop_mode_then(self, |c| c.handle_subscribe(parts));
                } else if eq_ignore_ascii(cmd, b"UNSUBSCRIBE") {
                    drop_mode_then(self, |c| c.handle_unsubscribe(parts));
                } else if eq_ignore_ascii(cmd, b"PSUBSCRIBE") {
                    drop_mode_then(self, |c| c.handle_psubscribe(parts));
                } else if eq_ignore_ascii(cmd, b"PUNSUBSCRIBE") {
                    drop_mode_then(self, |c| c.handle_punsubscribe(parts));
                } else if eq_ignore_ascii(cmd, b"PING") {
                    let out = &mut self.parser.wbuf;
                    let msg = parts.get(1).copied().unwrap_or("");
                    if msg.is_empty() {
                        out.extend_from_slice(b"*3\r\n$4\r\npong\r\n$0\r\n\r\n");
                    } else {
                        out.extend_from_slice(b"*3\r\n$4\r\npong\r\n");
                        resp::write_bulk(out, msg);
                    }
                } else if eq_ignore_ascii(cmd, b"RESET") || eq_ignore_ascii(cmd, b"QUIT") {
                    self.do_full_unsubscribe();
                    resp::write_simple(&mut self.parser.wbuf, "OK");
                } else {
                    self.parser.wbuf.extend_from_slice(
                        b"-ERR Command not allowed in subscribed state\r\n",
                    );
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Subscription handlers
    // -----------------------------------------------------------------------

    fn handle_subscribe(&mut self, parts: &[&str]) {
        if parts.len() < 2 {
            resp::write_wrong_args(&mut self.parser.wbuf, "subscribe");
            return;
        }
        let new_channels: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();
        let slot = self.ensure_slot();

        // Collect channels that are genuinely new (not already subscribed).
        // Do this inside a block so the mutable borrow of self.mode ends
        // before we call self.pubsub.subscribe().
        let to_register: Vec<String> = {
            let (ch_set, _) = self.sub_sets_mut();
            new_channels
                .iter()
                .filter(|ch| ch_set.insert((*ch).clone()))
                .cloned()
                .collect()
        };

        for ch in &to_register {
            self.pubsub.subscribe(ch, Arc::clone(&slot));
        }

        let total = self.sub_total();
        let out = &mut self.parser.wbuf;
        for (i, ch) in new_channels.iter().enumerate() {
            let count = total - new_channels.len() + i + 1;
            out.extend_from_slice(&encode_sub_reply("subscribe", ch, count));
        }
    }

    fn handle_unsubscribe(&mut self, parts: &[&str]) {
        let (slot, channels, patterns) = match &mut self.mode {
            ConnMode::Subscribed { slot, channels, patterns } => {
                (Arc::clone(slot), channels as *mut HashSet<String>, patterns as *mut HashSet<String>)
            }
            ConnMode::Normal => {
                self.parser.wbuf.extend_from_slice(&encode_sub_reply("unsubscribe", "", 0));
                return;
            }
        };
        // Safety: we only ever access channels/patterns through this conn on
        // one thread (the worker that owns it).
        let channels = unsafe { &mut *channels };
        let patterns = unsafe { &mut *patterns };

        let targets: Vec<String> = if parts.len() <= 1 {
            channels.iter().cloned().collect()
        } else {
            parts[1..].iter().map(|s| s.to_string()).collect()
        };

        let out = &mut self.parser.wbuf;
        let mut remaining = channels.len() + patterns.len();
        for ch in &targets {
            if channels.remove(ch) {
                self.pubsub.unsubscribe(ch, &slot);
                remaining -= 1;
                out.extend_from_slice(&encode_sub_reply("unsubscribe", ch, remaining));
            }
        }

        // Zero targets and zero subs → explicit empty reply
        if targets.is_empty() && channels.is_empty() {
            out.extend_from_slice(&encode_sub_reply("unsubscribe", "", 0));
        }

        if channels.is_empty() && patterns.is_empty() {
            self.mode = ConnMode::Normal;
        }
    }

    fn handle_psubscribe(&mut self, parts: &[&str]) {
        if parts.len() < 2 {
            resp::write_wrong_args(&mut self.parser.wbuf, "psubscribe");
            return;
        }
        let new_patterns: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();
        let slot = self.ensure_slot();

        // Same two-phase approach: collect new patterns with mutable borrow,
        // then register them with pubsub after the borrow ends.
        let to_register: Vec<String> = {
            let (_, pat_set) = self.sub_sets_mut();
            new_patterns
                .iter()
                .filter(|p| pat_set.insert((*p).clone()))
                .cloned()
                .collect()
        };

        for pat in &to_register {
            self.pubsub.psubscribe(pat, Arc::clone(&slot));
        }

        let total = self.sub_total();
        let out = &mut self.parser.wbuf;
        for (i, pat) in new_patterns.iter().enumerate() {
            let count = total - new_patterns.len() + i + 1;
            out.extend_from_slice(&encode_sub_reply("psubscribe", pat, count));
        }
    }

    fn handle_punsubscribe(&mut self, parts: &[&str]) {
        let (slot, channels, patterns) = match &mut self.mode {
            ConnMode::Subscribed { slot, channels, patterns } => {
                (Arc::clone(slot), channels as *mut HashSet<String>, patterns as *mut HashSet<String>)
            }
            ConnMode::Normal => {
                self.parser.wbuf.extend_from_slice(&encode_sub_reply("punsubscribe", "", 0));
                return;
            }
        };
        let channels = unsafe { &mut *channels };
        let patterns = unsafe { &mut *patterns };

        let targets: Vec<String> = if parts.len() <= 1 {
            patterns.iter().cloned().collect()
        } else {
            parts[1..].iter().map(|s| s.to_string()).collect()
        };

        let out = &mut self.parser.wbuf;
        let mut remaining = channels.len() + patterns.len();
        for pat in &targets {
            if patterns.remove(pat) {
                self.pubsub.punsubscribe(pat, &slot);
                remaining -= 1;
                out.extend_from_slice(&encode_sub_reply("punsubscribe", pat, remaining));
            }
        }

        if targets.is_empty() && patterns.is_empty() {
            out.extend_from_slice(&encode_sub_reply("punsubscribe", "", 0));
        }

        if channels.is_empty() && patterns.is_empty() {
            self.mode = ConnMode::Normal;
        }
    }

    fn do_full_unsubscribe(&mut self) {
        let (slot, channels, patterns) = match &mut self.mode {
            ConnMode::Subscribed { slot, channels, patterns } => (
                Arc::clone(slot),
                std::mem::take(channels),
                std::mem::take(patterns),
            ),
            ConnMode::Normal => return,
        };
        for ch in &channels {
            self.pubsub.unsubscribe(ch, &slot);
        }
        for pat in &patterns {
            self.pubsub.punsubscribe(pat, &slot);
        }
        self.mode = ConnMode::Normal;
    }

    // -----------------------------------------------------------------------
    // Sub-mode helpers
    // -----------------------------------------------------------------------

    /// Transition to Subscribed mode if not already, returning the slot.
    fn ensure_slot(&mut self) -> Arc<SubSlot> {
        if let ConnMode::Normal = self.mode {
            let slot = Arc::new(SubSlot::new(self.token, Arc::clone(&self.notifier)));
            self.mode = ConnMode::Subscribed {
                slot,
                channels: HashSet::new(),
                patterns: HashSet::new(),
            };
        }
        match &self.mode {
            ConnMode::Subscribed { slot, .. } => Arc::clone(slot),
            ConnMode::Normal => unreachable!(),
        }
    }

    /// Returns mutable refs to (channels, patterns).
    /// Only valid when mode == Subscribed.
    fn sub_sets_mut(&mut self) -> (&mut HashSet<String>, &mut HashSet<String>) {
        match &mut self.mode {
            ConnMode::Subscribed { channels, patterns, .. } => (channels, patterns),
            ConnMode::Normal => unreachable!("sub_sets_mut called in Normal mode"),
        }
    }

    fn sub_total(&self) -> usize {
        match &self.mode {
            ConnMode::Subscribed { channels, patterns, .. } => channels.len() + patterns.len(),
            ConnMode::Normal => 0,
        }
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        self.do_full_unsubscribe();
        self.store.client_disconnected();
    }
}

// ---------------------------------------------------------------------------
// Helper: some dispatch arms need to call a &mut self method while the match
// borrows self.mode.  This trampoline avoids the borrow conflict.
// ---------------------------------------------------------------------------
#[inline(always)]
fn drop_mode_then(conn: &mut Conn, f: impl FnOnce(&mut Conn)) {
    f(conn);
}

/// Allocation-free case-insensitive comparison for ASCII command names.
#[inline(always)]
fn eq_ignore_ascii(a: &[u8], upper: &[u8]) -> bool {
    if a.len() != upper.len() {
        return false;
    }
    a.iter()
        .zip(upper.iter())
        .all(|(&ac, &uc)| ac.to_ascii_uppercase() == uc)
}

// ---------------------------------------------------------------------------
// PUBSUB info command (inline here, not worth a separate file)
// ---------------------------------------------------------------------------
fn pubsub_info(parts: &[&str], pubsub: &Arc<PubSub>, out: &mut Vec<u8>) {
    let sub = match parts.get(1) {
        Some(s) => *s,
        None => {
            resp::write_err(out, "wrong number of arguments for 'pubsub' command");
            return;
        }
    };
    match sub.to_ascii_uppercase().as_str() {
        "CHANNELS" => {
            let pattern = parts.get(2).copied();
            let mut channels = pubsub.active_channels(pattern);
            channels.sort();
            resp::write_array(out, &channels);
        }
        "NUMSUB" => {
            let keys: Vec<&str> = parts[2..].to_vec();
            let pairs = pubsub.numsub(&keys);
            resp::write_array_header(out, pairs.len() * 2);
            for (ch, n) in pairs {
                resp::write_bulk(out, &ch);
                resp::write_integer(out, n as i64);
            }
        }
        "NUMPAT" => {
            resp::write_integer(out, pubsub.numpat() as i64);
        }
        _ => resp::write_err(out, "unknown pubsub subcommand"),
    }
}
