pub fn encode_message(channel: &str, message: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + channel.len() + message.len());
    out.extend_from_slice(b"*3\r\n$7\r\nmessage\r\n");
    bulk_into(&mut out, channel.as_bytes());
    bulk_into(&mut out, message.as_bytes());
    out
}

pub fn encode_pmessage(pattern: &str, channel: &str, message: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(48 + pattern.len() + channel.len() + message.len());
    out.extend_from_slice(b"*4\r\n$8\r\npmessage\r\n");
    bulk_into(&mut out, pattern.as_bytes());
    bulk_into(&mut out, channel.as_bytes());
    bulk_into(&mut out, message.as_bytes());
    out
}

pub fn encode_sub_reply(kind: &str, channel: &str, count: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + kind.len() + channel.len());
    out.extend_from_slice(b"*3\r\n");
    bulk_into(&mut out, kind.as_bytes());
    bulk_into(&mut out, channel.as_bytes());
    out.push(b':');
    push_usize(&mut out, count);
    out.extend_from_slice(b"\r\n");
    out
}

#[inline]
pub(crate) fn bulk_into(out: &mut Vec<u8>, b: &[u8]) {
    out.push(b'$');
    push_usize(out, b.len());
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(b);
    out.extend_from_slice(b"\r\n");
}

#[inline]
pub(crate) fn push_usize(out: &mut Vec<u8>, mut n: usize) {
    if n == 0 {
        out.push(b'0');
        return;
    }
    let start = out.len();
    while n > 0 {
        out.push(b'0' + (n % 10) as u8);
        n /= 10;
    }
    out[start..].reverse();
}
