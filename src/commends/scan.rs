use crate::storage::store::Store;

pub async fn scan(parts: Vec<String>, store: &Store) -> String {
    let args = match parts.as_slice() {
        [_, rest @ ..] if !rest.is_empty() => rest,
        _ => return "-ERR wrong number of arguments for 'scan' command\r\n".into(),
    };

    let cursor = match args[0].parse::<usize>() {
        Ok(c) => c,
        Err(_) => return "-ERR value is not an integer or out of range\r\n".into(),
    };

    let mut pattern: Option<&str> = None;
    let mut count: usize = 10;

    let mut i = 1;
    while i < args.len() {
        if args[i].eq_ignore_ascii_case("MATCH") {
            i += 1;
            if i >= args.len() {
                return "-ERR syntax error\r\n".into();
            }
            pattern = Some(&args[i]);
        } else if args[i].eq_ignore_ascii_case("COUNT") {
            i += 1;
            if i >= args.len() {
                return "-ERR syntax error\r\n".into();
            }
            count = match args[i].parse::<usize>() {
                Ok(c) if c > 0 => c,
                _ => return "-ERR value is not an integer or out of range\r\n".into(),
            };
        } else {
            return "-ERR syntax error\r\n".into();
        }
        i += 1;
    }

    let (next_cursor, keys) = store.scan(cursor, pattern, count);

    let cursor_str = next_cursor.to_string();
    let mut response = format!(
        "*2\r\n${}\r\n{}\r\n*{}\r\n",
        cursor_str.len(),
        cursor_str,
        keys.len()
    );
    for key in &keys {
        response.push_str(&format!("${}\r\n{}\r\n", key.len(), key));
    }
    response
}
