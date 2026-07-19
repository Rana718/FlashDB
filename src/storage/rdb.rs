use crate::storage::{
    store::Store,
    value::{FlashDB, StoreValue},
};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAGIC: &[u8; 4] = b"FLDB";
const VERSION: u8 = 1;
const TYPE_STRING: u8 = 0;
const TYPE_HASH: u8 = 1;
const TYPE_EOF: u8 = 0xFF;

pub fn save(store: &Store, path: &str) -> io::Result<()> {
    let tmp = format!("{path}.tmp");
    {
        let f = File::create(&tmp)?;
        let mut w = BufWriter::with_capacity(1 << 20, f);

        w.write_all(MAGIC)?;
        w.write_all(&[VERSION])?;

        let now_instant = Instant::now();
        let now_unix_ms = unix_ms_now();

        for entry in store.data.iter() {
            let key = entry.key();
            let val = entry.value();

            if val.is_expired() {
                continue;
            }

            let ttl_ms: u64 = match val.expires_at {
                None => 0,
                Some(exp) => {
                    let remaining = exp.saturating_duration_since(now_instant);
                    now_unix_ms + remaining.as_millis() as u64
                }
            };

            match &val.value {
                FlashDB::String(s) => {
                    write_u8(&mut w, TYPE_STRING)?;
                    write_u64(&mut w, ttl_ms)?;
                    write_bytes(&mut w, key.as_bytes())?;
                    write_bytes(&mut w, s.as_bytes())?;
                }
                FlashDB::Hash(h) => {
                    write_u8(&mut w, TYPE_HASH)?;
                    write_u64(&mut w, ttl_ms)?;
                    write_bytes(&mut w, key.as_bytes())?;
                    write_u32(&mut w, h.len() as u32)?;
                    for (f, v) in h {
                        write_bytes(&mut w, f.as_bytes())?;
                        write_bytes(&mut w, v.as_bytes())?;
                    }
                }
            }
        }

        write_u8(&mut w, TYPE_EOF)?;
        w.flush()?;
    }

    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn load(store: &Store, path: &str) -> io::Result<usize> {
    if !Path::new(path).exists() {
        return Ok(0);
    }

    let f = File::open(path)?;
    let mut r = BufReader::with_capacity(1 << 20, f);

    let mut magic = [0u8; 4];
    r.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid RDB magic",
        ));
    }
    let version = read_u8(&mut r)?;
    if version != VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported RDB version",
        ));
    }

    let now_unix_ms = unix_ms_now();
    let now_instant = Instant::now();
    let mut count = 0usize;

    loop {
        let type_byte = read_u8(&mut r)?;
        if type_byte == TYPE_EOF {
            break;
        }

        let ttl_ms = read_u64(&mut r)?;
        let key = read_string(&mut r)?;

        let expires_at: Option<Instant> = if ttl_ms == 0 {
            None
        } else if ttl_ms <= now_unix_ms {
            match type_byte {
                TYPE_STRING => {
                    read_string(&mut r)?;
                }
                TYPE_HASH => {
                    let n = read_u32(&mut r)? as usize;
                    for _ in 0..n {
                        read_string(&mut r)?;
                        read_string(&mut r)?;
                    }
                }
                _ => {}
            }
            continue;
        } else {
            let remaining_ms = ttl_ms - now_unix_ms;
            Some(now_instant + Duration::from_millis(remaining_ms))
        };

        let store_value = match type_byte {
            TYPE_STRING => {
                let val = read_string(&mut r)?;
                StoreValue::string_with_expiry(val, expires_at)
            }
            TYPE_HASH => {
                let n = read_u32(&mut r)? as usize;
                let mut h = HashMap::with_capacity(n);
                for _ in 0..n {
                    let field = read_string(&mut r)?;
                    let val = read_string(&mut r)?;
                    h.insert(field, val);
                }
                let mut sv = StoreValue::hash(h);
                sv.expires_at = expires_at;
                sv
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unknown type byte",
                ));
            }
        };

        store.data.insert(key, store_value);
        count += 1;
    }

    Ok(count)
}

pub fn start_background_save(store: Arc<Store>, path: String, interval: Duration) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(interval);
            match save(&store, &path) {
                Ok(()) => eprintln!("[rdb] saved to {path}"),
                Err(e) => eprintln!("[rdb] save error: {e}"),
            }
        }
    });
}

#[inline]
fn write_u8(w: &mut impl Write, v: u8) -> io::Result<()> {
    w.write_all(&[v])
}
#[inline]
fn write_u32(w: &mut impl Write, v: u32) -> io::Result<()> {
    w.write_all(&v.to_le_bytes())
}
#[inline]
fn write_u64(w: &mut impl Write, v: u64) -> io::Result<()> {
    w.write_all(&v.to_le_bytes())
}

#[inline]
fn write_bytes(w: &mut impl Write, b: &[u8]) -> io::Result<()> {
    write_u32(w, b.len() as u32)?;
    w.write_all(b)
}

#[inline]
fn read_u8(r: &mut impl Read) -> io::Result<u8> {
    let mut b = [0u8];
    r.read_exact(&mut b)?;
    Ok(b[0])
}
#[inline]
fn read_u32(r: &mut impl Read) -> io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}
#[inline]
fn read_u64(r: &mut impl Read) -> io::Result<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

fn read_string(r: &mut impl Read) -> io::Result<String> {
    let len = read_u32(r)? as usize;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    String::from_utf8(buf).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid utf8"))
}

fn unix_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
