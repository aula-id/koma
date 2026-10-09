//! HTTP adapters for external search providers. Never include response bodies or
//! request details in errors: providers can echo credentials in those fields.
use crate::model::web_search::{SearchProvider, WebSearchConfig};
use serde_json::{json, Value};
use std::time::Duration;

fn request(provider: SearchProvider, query: &str) -> (&'static str, Value) {
    match provider {
        SearchProvider::Firecrawl => (
            "https://api.firecrawl.dev/v2/search",
            json!({"query":query,"limit":8,"sources":["web"]}),
        ),
        SearchProvider::Tavily => (
            "https://api.tavily.com/search",
            json!({"query":query,"max_results":8,"search_depth":"basic","include_answer":false,"include_raw_content":false,"auto_parameters":false}),
        ),
        SearchProvider::Exa => (
            "https://api.exa.ai/search",
            json!({"query":query,"numResults":8,"type":"auto","contents":{"highlights":true}}),
        ),
        SearchProvider::BuiltIn => unreachable!(),
    }
}
fn check_status(provider: SearchProvider, status: u16) -> Result<(), String> {
    let label = provider.label();
    match status {
        401 | 403 => Err(format!(
            "{label}: authentication failed; check the saved API key"
        )),
        402 | 429 | 432 | 433 => Err(format!("{label}: quota or rate limit exceeded")),
        200..=299 => Ok(()),
        _ => Err(format!("{label}: response error (HTTP {status})")),
    }
}
fn parse(
    provider: SearchProvider,
    status: u16,
    body: &str,
) -> Result<Vec<super::web_search::SearchResult>, String> {
    let label = provider.label();
    check_status(provider, status)?;
    let malformed = || format!("{label}: malformed search response");
    let value: Value = serde_json::from_str(body).map_err(|_| malformed())?;
    if value.get("success") == Some(&Value::Bool(false)) {
        return Err(format!("{label}: search response reported failure"));
    }
    let rows = if provider == SearchProvider::Firecrawl {
        value.pointer("/data/web")
    } else {
        value.get("results")
    }
    .and_then(Value::as_array)
    .ok_or_else(malformed)?;
    rows.iter()
        .take(8)
        .map(|r| {
            let url = r
                .get("url")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(malformed)?;
            let title = r.get("title").and_then(Value::as_str).unwrap_or(url);
            let snippet = match provider {
                SearchProvider::Firecrawl => r
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                SearchProvider::Tavily => r
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                SearchProvider::Exa => match r.get("highlights") {
                    None | Some(Value::Null) => String::new(),
                    Some(Value::Array(items)) => items
                        .iter()
                        .map(|v| v.as_str().ok_or_else(malformed))
                        .collect::<Result<Vec<_>, _>>()?
                        .join(" "),
                    _ => return Err(malformed()),
                },
                _ => unreachable!(),
            };
            Ok(super::web_search::SearchResult {
                title: title.into(),
                url: url.into(),
                snippet,
            })
        })
        .collect()
}

fn perform(
    config: WebSearchConfig,
    query: String,
    endpoint: String,
    timeout: Duration,
) -> Result<String, String> {
    let provider = config.provider;
    let label = provider.label();
    if config.key(provider).is_empty() {
        return Err(format!(
            "{label}: missing API key; configure Web search settings"
        ));
    }
    let (_, payload) = request(provider, &query);
    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| format!("{label}: network client error"))?;
    let mut key = reqwest::header::HeaderValue::from_str(config.key(provider))
        .map_err(|_| format!("{label}: invalid API key header"))?;
    key.set_sensitive(true);
    let mut req = client.post(endpoint).json(&payload);
    if provider == SearchProvider::Exa {
        req = req.header("x-api-key", key);
    } else {
        let mut bearer =
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", config.key(provider)))
                .map_err(|_| format!("{label}: invalid API key header"))?;
        bearer.set_sensitive(true);
        req = req.header(reqwest::header::AUTHORIZATION, bearer);
    }
    let started = std::time::Instant::now();
    let response = req.send().map_err(|e| {
        if e.is_timeout() {
            format!("{label}: request timed out")
        } else {
            format!("{label}: network request failed")
        }
    })?;
    let status = response.status().as_u16();
    check_status(provider, status)?;
    // Limit response size without collecting an unbounded body.
    use std::io::Read;
    let mut bytes = Vec::new();
    response
        .take(5 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            if started.elapsed() >= timeout {
                format!("{label}: request timed out")
            } else {
                format!("{label}: response read failed")
            }
        })?;
    if bytes.len() > 5 * 1024 * 1024 {
        return Err(format!("{label}: search response too large"));
    }
    let body =
        std::str::from_utf8(&bytes).map_err(|_| format!("{label}: malformed search response"))?;
    let rows = parse(provider, status, body)?;
    if rows.is_empty() {
        return Ok(format!(
            "web_search: no results found for query: {query}\n({label} returned no results)"
        ));
    }
    Ok(super::web_search::format_results(&rows))
}

pub(super) fn search(config: WebSearchConfig, query: &str, region: &str) -> String {
    let provider = config.provider;
    let endpoint = request(provider, query).0.to_string();
    let query = query.to_string();
    // Blocking clients must be created and dropped outside a Tokio runtime.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(perform(config, query, endpoint, Duration::from_secs(30)));
    });
    let result = rx
        .recv_timeout(Duration::from_secs(35))
        .unwrap_or_else(|_| Err(format!("{}: request timed out", provider.label())));
    output(provider, region, result)
}
fn output(provider: SearchProvider, region: &str, result: Result<String, String>) -> String {
    let mut out = result.unwrap_or_else(|e| format!("error: {e}"));
    if !region.is_empty() && region != "wt-wt" {
        out.push_str(&format!(
            "\nNote: {} ignores the region parameter.",
            provider.label()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn external_error_and_region_note_without_fallback() {
        for p in SearchProvider::ALL.into_iter().skip(1) {
            let err = parse(p, 401, "secret").err().unwrap();
            let out = output(p, "us-en", Err(err));
            assert!(out.starts_with(&format!("error: {}: authentication failed", p.label())));
            assert!(out.contains("ignores the region parameter"));
            assert!(!out.contains("DuckDuckGo"));
            assert!(!output(p, "wt-wt", Ok("results".into())).contains("Note:"));
        }
    }
    #[test]
    fn normalization_errors_and_requests() {
        for p in SearchProvider::ALL.into_iter().skip(1) {
            let (_, payload) = request(p, "hello");
            assert_eq!(payload["query"], "hello");
            match p {
                SearchProvider::Firecrawl => {
                    assert_eq!(payload["limit"], 8);
                    assert_eq!(payload["sources"], json!(["web"]));
                    assert!(payload.get("scrapeOptions").is_none());
                }
                SearchProvider::Tavily => {
                    assert_eq!(payload["max_results"], 8);
                    assert_eq!(payload["search_depth"], "basic");
                    assert_eq!(payload["include_answer"], false);
                    assert_eq!(payload["include_raw_content"], false);
                }
                SearchProvider::Exa => {
                    assert_eq!(payload["numResults"], 8);
                    assert_eq!(payload["type"], "auto");
                    assert_eq!(payload["contents"], json!({"highlights":true}));
                }
                _ => unreachable!(),
            }
            for status in [401, 403, 402, 429, 432, 433, 500] {
                let err = parse(p, status, "secret").err().unwrap();
                assert!(!err.contains("secret"));
            }
            assert!(parse(p, 200, "bad json").is_err());
            let body = match p {
                SearchProvider::Firecrawl => {
                    r#"{"success":true,"data":{"web":[{"title":"Title","url":"https://example.com","description":"Snippet"}]}}"#
                }
                SearchProvider::Tavily => {
                    r#"{"results":[{"title":"Title","url":"https://example.com","content":"Snippet"}]}"#
                }
                _ => {
                    r#"{"results":[{"title":"Title","url":"https://example.com","highlights":["Snippet"]}]}"#
                }
            };
            let rows = parse(p, 200, body).unwrap();
            assert_eq!(rows[0].snippet, "Snippet");
            assert_eq!(rows[0].title, "Title");
            assert_eq!(rows[0].url, "https://example.com");
            let empty = if p == SearchProvider::Firecrawl {
                r#"{"data":{"web":[]}}"#
            } else {
                r#"{"results":[]}"#
            };
            assert!(parse(p, 200, empty).unwrap().is_empty());
            assert!(parse(p, 200, r#"{"results":[{}]}"#).is_err());
        }
    }
    #[test]
    fn mock_http_headers_payload_and_timeout() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        for p in SearchProvider::ALL.into_iter().skip(1) {
            let server = TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = format!("http://{}/search", server.local_addr().unwrap());
            let task = std::thread::spawn(move || {
                let (mut sock, _) = server.accept().unwrap();
                sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut chunk = [0; 1024];
                    let n = sock.read(&mut chunk).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&chunk[..n]);
                    if let Some(idx) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break idx + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
                let content_len: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .map(|n| n.parse().unwrap())
                    })
                    .unwrap();
                while bytes.len() < header_end + content_len {
                    let mut chunk = [0; 1024];
                    let n = sock.read(&mut chunk).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&chunk[..n]);
                }
                assert!(headers.contains(if p == SearchProvider::Exa {
                    "x-api-key: test-key"
                } else {
                    "authorization: Bearer test-key"
                }));
                let sent: Value = serde_json::from_slice(&bytes[header_end..]).unwrap();
                assert_eq!(sent, request(p, "hello").1);
                assert!(!sent.to_string().contains("test-key"));
                let body = if p == SearchProvider::Firecrawl {
                    r#"{"data":{"web":[]}}"#
                } else {
                    r#"{"results":[]}"#
                };
                write!(
                    sock,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            });
            let mut c = WebSearchConfig::default();
            c.provider = p;
            c.firecrawl_key.0 = "test-key".into();
            c.tavily_key.0 = "test-key".into();
            c.exa_key.0 = "test-key".into();
            assert!(perform(c, "hello".into(), endpoint, Duration::from_secs(2))
                .unwrap()
                .contains("no results"));
            task.join().unwrap();
        }
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", server.local_addr().unwrap());
        let task = std::thread::spawn(move || {
            let (_s, _) = server.accept().unwrap();
            std::thread::sleep(Duration::from_millis(100));
        });
        let mut c = WebSearchConfig::default();
        c.provider = SearchProvider::Exa;
        c.exa_key.0 = "key".into();
        assert!(
            perform(c, "hello".into(), endpoint, Duration::from_millis(20))
                .unwrap_err()
                .contains("timed out")
        );
        task.join().unwrap();
    }
}
