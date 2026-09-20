use super::{matching, CatalogModel};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::Mutex;

const CATALOG_URL: &str = "https://openrouter.ai/api/v1/models";
const TTL: Duration = Duration::from_secs(24 * 3600);
const RETRY: Duration = Duration::from_secs(300);
const MAX_BYTES: usize = 10 * 1024 * 1024;
#[derive(Default)]
struct Cache {
    models: Arc<Vec<CatalogModel>>,
    next_fetch: Option<Instant>,
    aliases: HashMap<String, (Instant, Option<CatalogModel>)>,
}
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
fn cache() -> &'static Mutex<Cache> {
    CACHE.get_or_init(|| Mutex::new(Cache::default()))
}
fn disk_path() -> Option<std::path::PathBuf> {
    crate::model::store::base_dir()
        .ok()
        .map(|p| p.join("cache/openrouter-context-models.json"))
}

#[derive(Deserialize)]
struct Envelope<T> {
    data: T,
}

// A dedicated header-less client: never route OAuth credentials to the catalog.
async fn fetch<T: serde::de::DeserializeOwned>(url: &str) -> anyhow::Result<T> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut response = client.get(url).send().await?.error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            bytes.len() + chunk.len() <= MAX_BYTES,
            "catalog response too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice::<Envelope<T>>(&bytes)?.data)
}

pub(super) async fn models() -> Arc<Vec<CatalogModel>> {
    let mut cache = cache().lock().await;
    if cache.next_fetch.is_some_and(|t| Instant::now() < t) {
        return cache.models.clone();
    }
    if cache.models.is_empty() {
        if let Some(path) = disk_path() {
            if let Ok(meta) = std::fs::metadata(&path) {
                if meta.len() <= MAX_BYTES as u64 {
                    if let Ok(bytes) = std::fs::read(&path) {
                        if let Ok(models) = serde_json::from_slice::<Vec<CatalogModel>>(&bytes) {
                            cache.models = Arc::new(models);
                            if meta
                                .modified()
                                .ok()
                                .and_then(|t| SystemTime::now().duration_since(t).ok())
                                .is_some_and(|age| age < TTL)
                            {
                                cache.next_fetch = Some(Instant::now() + RETRY);
                                return cache.models.clone();
                            }
                        }
                    }
                }
            }
        }
    }
    match fetch::<Vec<CatalogModel>>(CATALOG_URL).await {
        Ok(models) if !models.is_empty() => {
            if let Some(path) = disk_path() {
                if let (Some(parent), Ok(bytes)) = (path.parent(), serde_json::to_vec(&models)) {
                    let _ = std::fs::create_dir_all(parent);
                    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
                    if std::fs::write(&tmp, bytes).is_ok() {
                        let _ = std::fs::rename(&tmp, path);
                    }
                }
            }
            cache.models = Arc::new(models);
            cache.next_fetch = Some(Instant::now() + TTL);
            cache.aliases.clear();
        }
        _ => cache.next_fetch = Some(Instant::now() + RETRY),
    }
    cache.models.clone()
}

pub(super) async fn lookup_alias(query: &str, endpoint: &str) -> Option<CatalogModel> {
    let (author, slug) = match query.trim().split_once('/') {
        Some(parts) => parts,
        None => (matching::vendor(endpoint)?, query.trim()),
    };
    if author.is_empty() || slug.is_empty() || query.len() > 200 {
        return None;
    }
    let mut url = url::Url::parse("https://openrouter.ai/api/v1/model/").ok()?;
    url.path_segments_mut()
        .ok()?
        .pop_if_empty()
        .push(author)
        .push(slug);
    let key = url.to_string();
    let mut cache = cache().lock().await;
    if let Some((expires, model)) = cache.aliases.get(&key) {
        if Instant::now() < *expires {
            return model.clone();
        }
    }
    let model = fetch::<CatalogModel>(&key)
        .await
        .ok()
        .filter(|m| !m.id.trim().is_empty());
    if cache.aliases.len() >= 256 {
        cache.aliases.clear();
    }
    cache.aliases.insert(
        key,
        (
            Instant::now() + if model.is_some() { TTL } else { RETRY },
            model.clone(),
        ),
    );
    model
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[tokio::test]
    async fn public_fetch_has_no_authorization_and_does_not_follow_redirects() {
        for redirect in [false, true] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/models", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buf = [0u8; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                }
                let request = String::from_utf8(request).unwrap().to_lowercase();
                assert!(!request.contains("authorization:"));
                assert!(!request.contains("cookie:"));
                let body = r#"{"data":[{"id":"vendor/model-1","context_length":1000000,"top_provider":null}]}"#;
                let response = if redirect {
                    "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
                } else {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                };
                stream.write_all(response.as_bytes()).unwrap();
            });
            let result = fetch::<Vec<CatalogModel>>(&url).await;
            if redirect {
                // No request to the Location target: the empty redirect body
                // fails JSON parsing rather than yielding a connect failure.
                assert!(result.unwrap_err().is::<serde_json::Error>());
            } else {
                assert_eq!(result.unwrap()[0].window(), Some(1_000_000));
            }
            server.join().unwrap();
        }
    }
}
