use super::*;
use std::io::Cursor;
use std::path::PathBuf;

fn http(body: Vec<u8>) -> Vec<u8> {
    let mut msg = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    msg.extend(body);
    msg
}

fn chunk(payload: &[u8]) -> Vec<u8> {
    let mut out = format!("{:x}\r\n", payload.len()).into_bytes();
    out.extend(payload);
    out.extend(b"\r\n");
    out
}

fn read_events(bytes: &[u8]) -> Result<PullMeter, ApiError> {
    let meter = Mutex::new(PullMeter::new());
    let mut buf = Buf::new(Cursor::new(bytes.to_vec()));
    read_pull_response(&mut buf, &meter)?;
    Ok(meter
        .into_inner()
        .unwrap_or_else(|poison| poison.into_inner()))
}

#[test]
fn image_ref_splits_on_the_tag_after_the_last_slash() {
    assert_eq!(
        split_ref("ghcr.io/aula-id/koma-safety:0.4.10-minimal"),
        (
            "ghcr.io/aula-id/koma-safety".to_string(),
            "0.4.10-minimal".to_string()
        )
    );
    assert_eq!(
        split_ref("localhost:5000/koma"),
        ("localhost:5000/koma".to_string(), "latest".to_string())
    );
    assert_eq!(
        split_ref("ghcr.io/aula-id/koma-safety@sha256:abc"),
        (
            "ghcr.io/aula-id/koma-safety@sha256:abc".to_string(),
            String::new()
        )
    );
}

#[test]
fn pull_path_encodes_slashes() {
    assert_eq!(
        pull_path("ghcr.io/aula-id/koma-safety:0.4.10-minimal"),
        "/images/create?fromImage=ghcr.io%2Faula-id%2Fkoma-safety&tag=0.4.10-minimal"
    );
}

#[test]
fn unix_socket_host_keeps_the_absolute_path() {
    assert_eq!(
        unix_host_path("unix:///var/run/docker.sock").unwrap(),
        PathBuf::from("/var/run/docker.sock")
    );
    assert_eq!(
        unix_host_path("unix:///Users/me/.colima/default/docker.sock").unwrap(),
        PathBuf::from("/Users/me/.colima/default/docker.sock")
    );
    assert!(unix_host_path("tcp://127.0.0.1:2375").is_none());
}

#[test]
fn chunked_stream_survives_a_json_line_split_across_chunks() {
    let line = b"{\"status\":\"Status: Image is up to date for example:latest\"}\n";
    let mid = 18;
    let mut body = chunk(&line[..mid]);
    body.extend(chunk(&line[mid..]));
    body.extend(b"0\r\n\r\n");
    let meter = read_events(&http(body)).unwrap();
    assert!(meter.saw_status());
    assert!(meter.up_to_date());
}

#[test]
fn chunked_stream_sums_layer_bytes() {
    let events = [
        "{\"status\":\"Pulling fs layer\",\"id\":\"aaa\"}\n",
        "{\"status\":\"Downloading\",\"progressDetail\":{\"current\":40,\"total\":100},\"id\":\"aaa\"}\n",
        "{\"status\":\"Downloading\",\"progressDetail\":{\"current\":10,\"total\":50},\"id\":\"bbb\"}\n",
        "{\"status\":\"Status: Downloaded newer image for example:latest\"}\n",
    ];
    let mut body = Vec::new();
    for event in events {
        body.extend(chunk(event.as_bytes()));
    }
    body.extend(b"0\r\n\r\n");
    let meter = read_events(&http(body)).unwrap();
    assert!(meter.saw_status());
    assert!(!meter.up_to_date());
    assert_eq!(meter.transferred(), 50);
}

#[test]
fn manifest_unknown_is_not_retried_as_a_cli_pull() {
    let line = b"{\"error\":\"manifest unknown\"}\n";
    let mut body = chunk(line);
    body.extend(b"0\r\n\r\n");
    let err = read_events(&http(body)).unwrap_err();
    assert!(!err.fallback);
    assert!(err.message.contains("manifest unknown"));
}

#[test]
fn unauthorized_json_falls_back_to_the_cli() {
    let line = b"{\"error\":\"unauthorized: authentication required\"}\n";
    let mut body = chunk(line);
    body.extend(b"0\r\n\r\n");
    let err = read_events(&http(body)).unwrap_err();
    assert!(err.fallback, "{}", err.message);
}

#[test]
fn http_401_falls_back_to_the_cli() {
    let msg = b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 13\r\n\r\nunauthorized\n";
    let err = read_events(msg).unwrap_err();
    assert!(err.fallback, "{}", err.message);
    assert!(err.message.contains("401"));
}
