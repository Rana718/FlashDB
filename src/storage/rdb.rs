use crate::storage::{
    store::Store,
    value::{FlashDB, StoreValue, now_ms},
};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

const MAGIC: &[u8; 4] = b"FLDB";
const VERSION: u8 = 1;
const TYPE_STRING: u8 = 0;
const TYPE_HASH: u8 = 1;
const TYPE_EOF: u8 = 0xFF;

const MAX_LOAD_STRING: u32 = 512 * 1024 * 1024;

pub fn save(store: &Store, path: &str) -> io::Result<()> {
    let tmp = format!("{path}.tmp");
    {
        let f = File::create(&tmp)?;
        let mut w = BufWriter::with_capacity(1 << 20, f);

        w.write_all(MAGIC)?;
        w.write_all(&[VERSION])?;

        let mut write_result = Ok(());
        store.data.for_each(|key, val| {
            if write_result.is_err() {
                return;
            }
            if val.is_expired() {
                return;
            }

            let ttl_ms = val.expires_ms;

            match &val.value {
                FlashDB::String(s) => {
                    write_result = (|| {
                        write_u8(&mut w, TYPE_STRING)?;
                        write_u64(&mut w, ttl_ms)?;
                        write_bytes(&mut w, key.as_bytes())?;
                        write_bytes(&mut w, s.as_bytes())
                    })();
                }
                FlashDB::Hash(h) => {
                    write_result = (|| {
                        write_u8(&mut w, TYPE_HASH)?;
                        write_u64(&mut w, ttl_ms)?;
                        write_bytes(&mut w, key.as_bytes())?;
                        write_u32(&mut w, h.len() as u32)?;
                        for (f, v) in h.iter() {
                            write_bytes(&mut w, f.as_bytes())?;
                            write_bytes(&mut w, v.as_bytes())?;
                        }
                        Ok(())
                    })();
                }
            }
        });
        write_result?;

        write_u8(&mut w, TYPE_EOF)?;
        w.flush()?;

        let inner = w.into_inner().map_err(|e| e.into_error())?;
        fsync_file(&inner)?;
    }

    fs::rename(&tmp, path)?;

    if let Some(parent) = Path::new(path).parent()
        && let Ok(dir) = File::open(parent)
    {
        let _ = fsync_file(&dir);
    }

    Ok(())
}

pub fn load(store: &Store, path: &str) -> io::Result<usize> {
    if !Path::new(path).exists() {
        return Ok(0);
    }

    let f = File::open(path)?;
    let file_len = f.metadata()?.len();
    if file_len < 6 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "RDB file too short",
        ));
    }

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
        let type_byte = match read_u8(&mut r) {
            Ok(b) => b,
            Err(ref e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                eprintln!("[rdb] warning: truncated file, loaded {count} keys");
                break;
            }
            Err(e) => return Err(e),
        };

        if type_byte == TYPE_EOF {
            break;
        }

        let ttl_ms = read_u64(&mut r)?;
        let key = read_string_bounded(&mut r)?;

        if ttl_ms != 0 && ttl_ms <= now {
            match type_byte {
                TYPE_STRING => {
                    skip_string(&mut r)?;
                }
                TYPE_HASH => {
                    let n = read_u32(&mut r)? as usize;
                    for _ in 0..n {
                        skip_string(&mut r)?;
                        skip_string(&mut r)?;
                    }
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "unknown type byte in RDB",
                    ));
                }
            }
            continue;
        }

        let store_value = match type_byte {
            TYPE_STRING => {
                let val = read_string_bounded(&mut r)?;
                StoreValue {
                    value: FlashDB::String(val),
                    expires_ms: ttl_ms,
                }
            }
            TYPE_HASH => {
                let n = read_u32(&mut r)? as usize;
                if n > 10_000_000 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "hash field count too large",
                    ));
                }
                let mut h = HashMap::with_capacity(n.min(1024));
                for _ in 0..n {
                    let field = read_string_bounded(&mut r)?;
                    let val = read_string_bounded(&mut r)?;
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
    std::thread::Builder::new()
        .name("flashdb-rdb-saver".into())
        .spawn(move || {
            loop {
                std::thread::sleep(interval);
                match save(&store, &path) {
                    Ok(()) => {}
                    Err(e) => eprintln!("[rdb] background save error: {e}"),
                }
            }
        })
        .expect("failed to spawn RDB saver thread");
}

#[inline]
fn fsync_file(f: &File) -> io::Result<()> {
    let ret = unsafe { libc::fsync(f.as_raw_fd()) };
    if ret == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
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

fn read_string_bounded(r: &mut impl Read) -> io::Result<String> {
    let len = read_u32(r)?;
    if len > MAX_LOAD_STRING {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "string length exceeds maximum in RDB",
        ));
    }
    let len = len as usize;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    String::from_utf8(buf).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid utf8"))
}

fn skip_string(r: &mut impl Read) -> io::Result<()> {
    let len = read_u32(r)? as u64;
    let mut remaining = len;
    let mut skip_buf = [0u8; 8192];
    while remaining > 0 {
        let chunk = remaining.min(8192) as usize;
        r.read_exact(&mut skip_buf[..chunk])?;
        remaining -= chunk as u64;
    }
    Ok(())
}
