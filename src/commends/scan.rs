use crate::storage::store::Store;
use crate::utils::resp;

pub fn scan(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    let args = match parts {
        [_, rest @ ..] if !rest.is_empty() => rest,
        _ => {
            resp::write_wrong_args(out, "scan");
            return;
        }
    };

    let cursor = match args[0].parse::<usize>() {
        Ok(c) => c,
        Err(_) => {
            resp::write_err(out, "value is not an integer or out of range");
            return;
        }
    };

    let mut pattern: Option<&str> = None;
    let mut count: usize = 10;

    let mut i = 1;
    while i < args.len() {
        match args[i].to_ascii_uppercase().as_str() {
            "MATCH" => {
                i += 1;
                if i >= args.len() {
                    resp::write_err(out, "syntax error");
                    return;
                }
                pattern = Some(&args[i]);
            }
            "COUNT" => {
                i += 1;
                if i >= args.len() {
                    resp::write_err(out, "syntax error");
                    return;
                }
                count = match args[i].parse::<usize>() {
                    Ok(c) if c > 0 => c,
                    _ => {
                        resp::write_err(out, "value is not an integer or out of range");
                        return;
                    }
                };
            }
            _ => {
                resp::write_err(out, "syntax error");
                return;
            }
        }
        i += 1;
    }

    let (next_cursor, keys) = store.scan(cursor, pattern, count);

    out.extend_from_slice(b"*2\r\n");
    let cursor_str = next_cursor.to_string();
    resp::write_bulk(out, &cursor_str);
    resp::write_array(out, &keys);
}
