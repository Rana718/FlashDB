use crate::utils::resp;
use crate::storage::store::Store;

pub async fn scan(parts: Vec<String>, store: &Store) -> String {
    let args = match parts.as_slice() {
        [_, rest @ ..] if !rest.is_empty() => rest,
        _ => return resp::wrong_args("scan"),
    };

    let cursor = match args[0].parse::<usize>() {
        Ok(c) => c,
        Err(_) => return resp::err("value is not an integer or out of range"),
    };

    let mut pattern: Option<&str> = None;
    let mut count: usize = 10;

    let mut i = 1;
    while i < args.len() {
        match args[i].to_ascii_uppercase().as_str() {
            "MATCH" => {
                i += 1;
                if i >= args.len() { return resp::err("syntax error"); }
                pattern = Some(&args[i]);
            }
            "COUNT" => {
                i += 1;
                if i >= args.len() { return resp::err("syntax error"); }
                count = match args[i].parse::<usize>() {
                    Ok(c) if c > 0 => c,
                    _ => return resp::err("value is not an integer or out of range"),
                };
            }
            _ => return resp::err("syntax error"),
        }
        i += 1;
    }

    let (next_cursor, keys) = store.scan(cursor, pattern, count);

    let cursor_str = next_cursor.to_string();
    let mut out = format!("*2\r\n${}\r\n{}\r\n*{}\r\n", cursor_str.len(), cursor_str, keys.len());
    for key in &keys {
        out.push_str(&resp::bulk(key));
    }
    out
}
