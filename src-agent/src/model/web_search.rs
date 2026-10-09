//! Global search preferences. Secrets are write-only on settings transports.
use super::app_config::AppConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchProvider {
    #[default]
    BuiltIn,
    Firecrawl,
    Tavily,
    Exa,
}
impl SearchProvider {
    pub const ALL: [Self; 4] = [Self::BuiltIn, Self::Firecrawl, Self::Tavily, Self::Exa];
    pub fn label(self) -> &'static str {
        match self {
            Self::BuiltIn => "Built-in DuckDuckGo",
            Self::Firecrawl => "Firecrawl",
            Self::Tavily => "Tavily",
            Self::Exa => "Exa",
        }
    }
    /// Short chip suffix for the tool-result box (`web · default`).
    pub fn chip(self) -> &'static str {
        match self {
            Self::BuiltIn => "default",
            Self::Firecrawl => "firecrawl",
            Self::Tavily => "tavily",
            Self::Exa => "exa",
        }
    }
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SearchKey(pub String);
impl std::fmt::Debug for SearchKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WebSearchConfig {
    pub provider: SearchProvider,
    pub firecrawl_key: SearchKey,
    pub tavily_key: SearchKey,
    pub exa_key: SearchKey,
}
impl WebSearchConfig {
    pub fn key(&self, provider: SearchProvider) -> &str {
        match provider {
            SearchProvider::BuiltIn => "",
            SearchProvider::Firecrawl => &self.firecrawl_key.0,
            SearchProvider::Tavily => &self.tavily_key.0,
            SearchProvider::Exa => &self.exa_key.0,
        }
        .trim()
    }
    pub fn status(&self) -> SearchStatus {
        SearchStatus {
            provider: self.provider,
            saved_keys: SearchProvider::ALL
                .iter()
                .skip(1)
                .copied()
                .filter(|p| !self.key(*p).is_empty())
                .collect(),
        }
    }
    fn activate(&mut self, provider: SearchProvider, key: Option<SearchKey>) -> Result<(), String> {
        if provider != SearchProvider::BuiltIn {
            if let Some(key) = key {
                let trimmed = key.0.trim().to_string();
                if trimmed.is_empty() {
                    return Err(format!("{} requires an API key", provider.label()));
                }
                let slot = match provider {
                    SearchProvider::Firecrawl => &mut self.firecrawl_key,
                    SearchProvider::Tavily => &mut self.tavily_key,
                    SearchProvider::Exa => &mut self.exa_key,
                    _ => unreachable!(),
                };
                *slot = SearchKey(trimmed);
            }
            if self.key(provider).is_empty() {
                return Err(format!("{} requires an API key", provider.label()));
            }
        }
        self.provider = provider;
        Ok(())
    }
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchStatus {
    pub provider: SearchProvider,
    pub saved_keys: Vec<SearchProvider>,
}

pub fn read_global_config() -> Result<Option<AppConfig>, String> {
    let path = super::store::base_dir()
        .map_err(|_| "Could not resolve web search configuration".to_string())?
        .join("config.json");
    read_config_at(&path)
}
fn read_config_at(path: &std::path::Path) -> Result<Option<AppConfig>, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| "Could not read web search configuration".to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("Could not read web search configuration".to_string()),
    }
}

/// Persist a candidate before publishing it in memory. Shared by GUI and TUI.
pub fn save_selection(
    config: &mut AppConfig,
    provider: SearchProvider,
    key: Option<SearchKey>,
) -> Result<(), String> {
    // Refresh the authoritative file before changing it. Never turn an unreadable
    // configuration into an empty catalogue or lose keys saved by another session.
    let mut current = read_global_config()?.unwrap_or_else(|| config.clone());
    commit_selection(&mut current, provider, key, |c| {
        crate::app::runtime::save_web_search_config(c)
            .map_err(|_| "Could not save web search settings".to_string())
    })?;
    *config = current;
    Ok(())
}
pub(crate) fn commit_selection(
    config: &mut AppConfig,
    provider: SearchProvider,
    key: Option<SearchKey>,
    persist: impl FnOnce(&AppConfig) -> Result<(), String>,
) -> Result<(), String> {
    let mut candidate = config.clone();
    candidate.web_search.activate(provider, key)?;
    persist(&candidate)?;
    *config = candidate;
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchEditor {
    pub provider: SearchProvider,
    pub key: SearchKey,
    pub error: Option<String>,
}
impl SearchEditor {
    pub fn masked(&self) -> Self {
        Self {
            provider: self.provider,
            key: SearchKey("•".repeat(self.key.0.chars().count())),
            error: self.error.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chip_names() {
        assert_eq!(SearchProvider::BuiltIn.chip(), "default");
        assert_eq!(SearchProvider::Tavily.chip(), "tavily");
        assert_eq!(SearchProvider::Firecrawl.chip(), "firecrawl");
        assert_eq!(SearchProvider::Exa.chip(), "exa");
    }
    #[test]
    fn authoritative_search_config_reads_every_time_and_rejects_corruption() {
        let dir = std::env::temp_dir().join(format!("koma-search-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        assert!(read_config_at(&path).unwrap().is_none());
        std::fs::write(&path, "{}").unwrap();
        let mut c = read_config_at(&path).unwrap().unwrap();
        assert_eq!(c.web_search.provider, SearchProvider::BuiltIn);
        commit_selection(
            &mut c,
            SearchProvider::Exa,
            Some(SearchKey("secret".into())),
            |candidate| {
                super::super::memory::atomic_write(&path, &serde_json::to_vec(candidate).unwrap())
                    .map_err(|_| "save failed".into())
            },
        )
        .unwrap();
        // New invocations (including independent sessions/subagents) read the
        // freshly persisted choice rather than any previous config snapshot.
        assert_eq!(
            read_config_at(&path).unwrap().unwrap().web_search.provider,
            SearchProvider::Exa
        );
        std::fs::write(&path, "invalid config containing secret").unwrap();
        let error = read_config_at(&path).unwrap_err();
        assert!(!error.contains("secret"));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn old_config_and_roundtrip() {
        let mut c: AppConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(c.web_search.provider, SearchProvider::BuiltIn);
        c.web_search
            .activate(SearchProvider::Exa, Some(SearchKey(" secret ".into())))
            .unwrap();
        c.web_search
            .activate(SearchProvider::Tavily, Some(SearchKey("other".into())))
            .unwrap();
        c.web_search
            .activate(SearchProvider::BuiltIn, None)
            .unwrap();
        let c: AppConfig = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
        assert_eq!(c.web_search.key(SearchProvider::Exa), "secret");
        assert_eq!(c.web_search.status().saved_keys.len(), 2);
        assert!(!format!("{c:?}").contains("secret"));
        assert!(!serde_json::to_string(&c.web_search.status())
            .unwrap()
            .contains("secret"));
    }
    #[test]
    fn failure_keeps_previous_selection_and_keys() {
        let mut c = AppConfig::default();
        assert!(commit_selection(&mut c, SearchProvider::Exa, None, |_| Ok(())).is_err());
        assert!(commit_selection(
            &mut c,
            SearchProvider::Exa,
            Some(SearchKey("secret".into())),
            |_| Err("disk full".into())
        )
        .is_err());
        assert_eq!(c.web_search.provider, SearchProvider::BuiltIn);
        assert!(c.web_search.exa_key.0.is_empty());
        commit_selection(
            &mut c,
            SearchProvider::Exa,
            Some(SearchKey("secret".into())),
            |_| Ok(()),
        )
        .unwrap();
        commit_selection(&mut c, SearchProvider::BuiltIn, None, |_| Ok(())).unwrap();
        commit_selection(&mut c, SearchProvider::Exa, None, |_| Ok(())).unwrap();
        assert_eq!(c.web_search.provider, SearchProvider::Exa);
    }
}
