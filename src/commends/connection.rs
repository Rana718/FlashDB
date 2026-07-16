pub async fn ping(parts: Vec<String>) -> String {
    match parts.as_slice(){
       [_] => "+PONG\r\n".to_string(),
       _ => "ERR wrong number of arguments for 'ping' command\r\n".to_string(),
    }
}

pub async fn echo(parts: Vec<String>) -> String {
    match parts.as_slice(){
        [_] => parts[0].clone(),
        _ => "ERR wrong number of arguments for 'echo' command\r\n".to_string(),
    }
}