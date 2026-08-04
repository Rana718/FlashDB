use crate::storage::{
    store::Store,
    value::{FlashDB, StoreValue, now_ms},
};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

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

        store.data.for_each(|key, val| {
            if val.is_expired() {
                return;
            }

            let ttl_ms = val.expires_ms;

            match &val.value {
                FlashDB::String(s) => {
                    let _ = write_u8(&mut w, TYPE_STRING);
                    let _ = write_u64(&mut w, ttl_ms);
                    let _ = write_bytes(&mut w, key.as_bytes());
                    let _ = write_bytes(&mut w, s.as_bytes());
                }
                FlashDB::Hash(h) => {
                    let _ = write_u8(&mut w, TYPE_HASH);
                    let _ = write_u64(&mut w, ttl_ms);
                    let _ = write_bytes(&mut w, key.as_bytes());
                    let _ = write_u32(&mut w, h.len() as u32);
                    for (f, v) in h.iter() {
                        let _ = write_bytes(&mut w, f.as_bytes());
                        let _ = write_bytes(&mut w, v.as_bytes());
                    }
                }
            }
        });

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

    let now = now_ms();
    let mut count = 0usize;

    loop {
        let type_byte = read_u8(&mut r)?;
        if type_byte == TYPE_EOF {
            break;
        }

        let ttl_ms = read_u64(&mut r)?;
        let key = read_string(&mut r)?;

        if ttl_ms != 0 && ttl_ms <= now {
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
        }

        let store_value = match type_byte {
            TYPE_STRING => {
                let val = read_string(&mut r)?;
                StoreValue {
                    value: FlashDB::String(val),
                    expires_ms: ttl_ms,
                }
            }
            TYPE_HASH => {
                let n = read_u32(&mut r)? as usize;
                let mut h = HashMap::with_capacity(n);
                for _ in 0..n {
                    let field = read_string(&mut r)?;
                    let val = read_string(&mut r)?;
                    h.insert(field, val);
                }
                StoreValue {
                    value: FlashDB::Hash(Box::new(h)),
                    expires_ms: ttl_ms,
                }
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
            if let Err(e) = save(&store, &path) {
                eprintln!("[rdb] save error: {e}");
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
