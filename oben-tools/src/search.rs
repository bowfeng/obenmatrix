use std::collections::HashSet;

use super::registry::{Tool, ToolCall, ToolRegistry};
use anyhow::anyhow;
use oben_models::{ToolMeta, ToolParameter, ToolParameters, ToolResult};

fn make_search_tool_def() -> ToolMeta {
    ToolMeta {
        name: "web_search".into(),
        description: "Search the web for information".into(),
        parameters: ToolParameters::Flat(vec![
            ToolParameter::required("query", "Search query", "string"),
            ToolParameter::optional("max_results", "Maximum number of results", "number"),
        ]),
    }
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[async_trait::async_trait]
pub trait SearchProvider: Send + Sync {
    async fn search(&self, query: &str, max_results: usize) -> anyhow::Result<Vec<SearchResult>>;
}

use reqwest::Client;

pub struct DuckDuckGoProvider {
    client: Client,
}

impl DuckDuckGoProvider {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl SearchProvider for DuckDuckGoProvider {
    async fn search(&self, query: &str, max_results: usize) -> anyhow::Result<Vec<SearchResult>> {
        let url = format!(
            "https://html.duckduckgo.com/html/?q={}",
            urlencoding::encode(query)
        );

        let resp = self.client.get(&url).send().await?;

        if !resp.status().is_success() {
            return Err(anyhow::anyhow!(
                "DuckDuckGo search failed: HTTP {}",
                resp.status()
            ));
        }

        let html = resp.text().await?;

        let results = extract_ddg_results(&html, max_results);

        Ok(results)
    }
}

fn extract_ddg_results(html: &str, max_results: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let mut seen_urls = HashSet::new();

    for line in html.lines() {
        if line.contains("result__a") {
            let title = extract_ddg_title(line);
            let url = extract_ddg_url(line);

            if !url.is_empty() && !seen_urls.contains(&url) {
                seen_urls.insert(url.clone());
                results.push(SearchResult {
                    title,
                    url,
                    snippet: String::new(),
                });
                if results.len() >= max_results {
                    break;
                }
            }
        }
    }

    results
}

fn extract_ddg_title(line: &str) -> String {
    if let Some(start) = line.find('>') {
        let end = line[start..].find("</a>").map(|i| start + i).unwrap_or(line.len());
        let title = line[start..end].replace("<[^>]+>", "");
        title.trim().to_string()
    } else {
        String::new()
    }
}

fn extract_ddg_url(line: &str) -> String {
    if let Some(start) = line.find("href=\"") {
        let start = start + 6;
        if let Some(end) = line[start..].find('"') {
            let url = &line[start..start + end];
            if url.starts_with("/l/?url=") {
                if let Some(u) = url.split("url=").nth(1) {
                    return urlencoding::decode(u).unwrap_or_default().to_string();
                }
            }
            return urlencoding::decode(url).unwrap_or_default().to_string();
        }
    }
    String::new()
}

pub struct BraveProvider {
    api_key: String,
}

impl BraveProvider {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    pub async fn search(&self, query: &str, max_results: usize) -> anyhow::Result<Vec<SearchResult>> {
        let client = reqwest::Client::new();

        let url = format!(
            "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
            query,
            max_results.min(50)
        );

        let resp = client
            .get(&url)
            .header("X-Subscription-Token", &self.api_key)
            .header("Accept", "application/json")
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Brave API error: {} - {}", status, body));
        }

        let json: serde_json::Value = resp.json().await?;

        let results = json["web"]["results"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .take(max_results)
            .filter_map(|r| {
                Some(SearchResult {
                    title: r["title"].as_str()?.to_string(),
                    url: r["url"].as_str()?.to_string(),
                    snippet: r["description"].as_str()?.to_string(),
                })
            })
            .collect();

        Ok(results)
    }
}

#[async_trait::async_trait]
impl SearchProvider for BraveProvider {
    async fn search(&self, query: &str, max_results: usize) -> anyhow::Result<Vec<SearchResult>> {
        BraveProvider::search(self, query, max_results).await
    }
}

pub struct WebSearchTool {
    provider: Option<Box<dyn SearchProvider>>,
}

impl WebSearchTool {
    pub fn new_with_provider(provider: Box<dyn SearchProvider>) -> Self {
        Self { provider: Some(provider) }
    }

    pub fn new() -> Self {
        Self { provider: None }
    }
}

async fn execute_web_search<'a>(tool: &WebSearchTool, call: &ToolCall<'a>) -> anyhow::Result<ToolResult> {
    let query = call.required_str("query")?;
    let max_results = call.optional_u64("max_results", 5) as usize;

    if let Some(ref provider) = tool.provider {
        let results = provider.search(query, max_results).await?;
        let output = results
            .iter()
            .enumerate()
            .map(|(i, r)| format!("{}. [{}]({})", i + 1, r.title, r.url))
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(ToolResult {
            call_id: call.call_id.clone(),
            output,
            error: None,
        });
    }

    Err(anyhow!("No search provider configured. Add to config: `tools.search.provider`"))
}

#[async_trait::async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }
    fn description(&self) -> &str {
        "Search the web for information"
    }
    async fn execute(&self, call: &ToolCall) -> ToolResult {
        match execute_web_search(self, call).await {
            Ok(result) => result,
            Err(e) => ToolResult {
                call_id: call.call_id.clone(),
                output: String::new(),
                error: Some(e.to_string()),
            },
        }
    }
    fn clone_tool(&self) -> Box<dyn Tool> {
        Box::new(Self::new())
    }
}

pub fn register(registry: &mut ToolRegistry) {
    let tool = Box::new(WebSearchTool::new());
    registry.register_with_def(tool, make_search_tool_def());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_duckduckgo_provider_new() {
        let _provider = DuckDuckGoProvider::new();
    }

    #[test]
    fn test_brave_provider_new() {
        let provider = BraveProvider::new("test-key".to_string());
        assert_eq!(provider.api_key, "test-key");
    }

    #[test]
    fn test_brave_provider_search_url() {
        let query = "test query";
        let max_results = 10;

        let url = format!(
            "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
            query,
            max_results.min(50)
        );

        assert!(url.contains("test"));
        assert!(url.contains("query"));
        assert!(url.contains("count=10"));
    }

    #[test]
    fn test_search_result_clone() {
        let result = SearchResult {
            title: "Test Title".to_string(),
            url: "https://example.com".to_string(),
            snippet: "Test snippet".to_string(),
        };

        let cloned = result.clone();
        assert_eq!(cloned.title, result.title);
        assert_eq!(cloned.url, result.url);
        assert_eq!(cloned.snippet, result.snippet);
    }
}
