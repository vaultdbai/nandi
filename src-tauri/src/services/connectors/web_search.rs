//! The `web_search` connector: search the web and read pages, without an
//! API key, only on the sites the calling agent may reach (its skill.md
//! `network.allow`, narrowed by its policies).
//!
//! - `search` asks the search engine's own results page (Bing's RSS
//!   results by default, or DuckDuckGo's HTML page), limited to the
//!   agent's sites with `site:`, and drops any result outside them
//! - `read_page` fetches a page on one of those sites as plain text;
//!   every redirect must stay on them, and this computer's own addresses
//!   are never fetched
//!
//! The agent's sites arrive as its connector options' `allowed_sites`,
//! which Nandi sets on every call ([`ALLOWED_SITES`]); an agent's own
//! skill.md can't widen them.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use futures_util::future::BoxFuture;
use regex::Regex;
use serde_json::{json, Value};

use super::{Connector, ToolDef};
use crate::services::egress::capabilities::{Capabilities, HostRule, NetworkCapabilities};

/// The options key Nandi fills with the agent's allowed sites
pub const ALLOWED_SITES: &str = "allowed_sites";
const DEFAULT_COUNT: usize = 5;
const MAX_COUNT: usize = 20;
/// `site:` terms in one query; more sites search only the first ones
const MAX_SITE_TERMS: usize = 10;
/// Characters of a page `read_page` returns
const MAX_PAGE_CHARS: usize = 20_000;
const MAX_PAGE_BYTES: usize = 5 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
const TIMEOUT: Duration = Duration::from_secs(20);
const USER_AGENT: &str = concat!(
    "Mozilla/5.0 (compatible; Nandi/",
    env!("CARGO_PKG_VERSION"),
    ")"
);

/// Which results page `search` reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// `https://www.bing.com/search?format=rss`
    Bing,
    /// `https://html.duckduckgo.com/html/`
    DuckDuckGo,
}

impl Engine {
    /// `config.engine`: `bing` (default) or `duckduckgo`
    pub fn parse(name: Option<&str>) -> Result<Self, String> {
        match name.map(|n| n.trim().to_ascii_lowercase()).as_deref() {
            None | Some("") | Some("bing") => Ok(Self::Bing),
            Some("duckduckgo") | Some("ddg") => Ok(Self::DuckDuckGo),
            Some(other) => Err(format!(
                "Unknown search engine '{other}': use bing or duckduckgo"
            )),
        }
    }

    fn endpoint(self) -> &'static str {
        match self {
            Self::Bing => "https://www.bing.com/search",
            Self::DuckDuckGo => "https://html.duckduckgo.com/html/",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Bing => "Bing",
            Self::DuckDuckGo => "DuckDuckGo",
        }
    }
}

pub struct WebSearchConnector {
    engine: Engine,
    endpoint: String,
}

/// One search result.
#[derive(Debug, Clone, PartialEq)]
struct Hit {
    title: String,
    url: String,
    description: String,
}

fn search_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(TIMEOUT)
            .build()
            .expect("failed to build HTTP client")
    })
}

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("valid regex")
}

impl WebSearchConnector {
    /// `endpoint` overrides the engine's URL (tests).
    pub fn new(engine: Engine, endpoint: Option<String>) -> Self {
        Self {
            engine,
            endpoint: endpoint.unwrap_or_else(|| engine.endpoint().to_string()),
        }
    }

    pub fn tool_defs() -> Vec<ToolDef> {
        vec![
            ToolDef {
                name: "search".into(),
                description: "Search the web, on the sites this agent may reach. Returns titles, URLs, and snippets.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"},
                        "count": {"type": "integer", "minimum": 1, "maximum": MAX_COUNT},
                    },
                    "required": ["query"],
                }),
                sensitive: false,
            },
            ToolDef {
                name: "read_page".into(),
                description: "Read a web page on a site this agent may reach, as plain text.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {"url": {"type": "string"}},
                    "required": ["url"],
                }),
                sensitive: false,
            },
        ]
    }

    async fn search(&self, args: &Value, sites: &Sites) -> Result<Value, String> {
        let query = string_arg(args, "query")?;
        let count = args
            .get("count")
            .and_then(Value::as_u64)
            .map_or(DEFAULT_COUNT, |c| c as usize)
            .clamp(1, MAX_COUNT);
        let query = sites.scoped_query(query)?;
        let name = self.engine.label();
        let url = match self.engine {
            Engine::Bing => reqwest::Url::parse_with_params(
                &self.endpoint,
                &[
                    ("q", query.as_str()),
                    ("format", "rss"),
                    ("count", &MAX_COUNT.to_string()),
                ],
            ),
            Engine::DuckDuckGo => {
                reqwest::Url::parse_with_params(&self.endpoint, &[("q", query.as_str())])
            }
        }
        .map_err(|e| format!("Invalid {name} address: {e}"))?;
        let response = search_client()
            .get(url)
            .send()
            .await
            .map_err(|e| format!("Could not reach {name}: {}", e.without_url()))?;
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(format!("{name} is limiting searches; try again later"));
        }
        if !status.is_success() {
            return Err(format!("{name} search failed ({status})"));
        }
        let body = response
            .text()
            .await
            .map_err(|e| format!("Could not read {name}'s results: {e}"))?;
        let hits = match self.engine {
            Engine::Bing => parse_bing_rss(&body),
            Engine::DuckDuckGo => parse_duckduckgo(&body),
        };
        let results: Vec<Value> = hits
            .into_iter()
            .filter(|hit| sites.allows_url(&hit.url))
            .take(count)
            .map(|hit| json!({"title": hit.title, "url": hit.url, "description": hit.description}))
            .collect();
        Ok(json!({ "results": results }))
    }

    async fn read_page(&self, args: &Value, sites: &Sites) -> Result<Value, String> {
        let url = reqwest::Url::parse(string_arg(args, "url")?)
            .map_err(|_| "'url' must be a full web address (https://…)".to_string())?;
        sites.check_url(&url)?;
        let redirects = sites.clone();
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() >= MAX_REDIRECTS {
                    attempt.error("too many redirects")
                } else if redirects.check_url(attempt.url()).is_err() {
                    attempt.error("redirected to a site this agent may not reach")
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(|e| format!("Could not set up the request: {e}"))?;
        let response = client
            .get(url.clone())
            .send()
            .await
            .map_err(|e| format!("Could not read {url}: {}", e.without_url()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("{url} answered {status}"));
        }
        let final_url = response.url().to_string();
        let is_html = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_none_or(|v| v.contains("html"));
        let bytes = response
            .bytes()
            .await
            .map_err(|e| format!("Could not read {url}: {}", e.without_url()))?;
        let raw = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_PAGE_BYTES)]);
        let (title, text) = if is_html {
            (page_title(&raw), html_text(&raw))
        } else {
            (String::new(), raw.trim().to_string())
        };
        let truncated = text.chars().count() > MAX_PAGE_CHARS;
        let text: String = text.chars().take(MAX_PAGE_CHARS).collect();
        Ok(json!({ "url": final_url, "title": title, "text": text, "truncated": truncated }))
    }
}

impl Connector for WebSearchConnector {
    fn list_tools(&self) -> Vec<ToolDef> {
        Self::tool_defs()
    }

    /// Without an agent there are no allowed sites, so every call is refused
    fn call_tool<'a>(
        &'a self,
        name: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, String>> {
        self.call_tool_for(name, args, None)
    }

    fn call_tool_for<'a>(
        &'a self,
        name: &'a str,
        args: &'a Value,
        options: Option<&'a serde_yaml::Value>,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            let sites = Sites::from_options(options);
            match name {
                "search" => self.search(args, &sites).await,
                "read_page" => self.read_page(args, &sites).await,
                other => Err(format!("Unknown tool '{other}'")),
            }
        })
    }

    /// Needs no key; doesn't search (that would cost the engine a query)
    fn health_check(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
}

/// The sites an agent may search and read: its `network.allow` rules.
#[derive(Clone)]
struct Sites {
    rules: Arc<Capabilities>,
}

impl Sites {
    fn from_options(options: Option<&serde_yaml::Value>) -> Self {
        let allow: Vec<HostRule> = options
            .and_then(|o| o.get(ALLOWED_SITES))
            .and_then(|v| serde_yaml::from_value(v.clone()).ok())
            .unwrap_or_default();
        Self {
            rules: Arc::new(Capabilities {
                network: NetworkCapabilities { allow },
                ..Capabilities::default()
            }),
        }
    }

    fn rules(&self) -> &[HostRule] {
        &self.rules.network.allow
    }

    fn any_site(&self) -> bool {
        self.rules().iter().any(|rule| rule.host == "*")
    }

    /// `query` limited to the sites with `site:` terms (Bing and
    /// DuckDuckGo read `OR`); unchanged when every site is allowed
    fn scoped_query(&self, query: &str) -> Result<String, String> {
        if self.rules().is_empty() {
            return Err(no_sites());
        }
        if self.any_site() {
            return Ok(query.to_string());
        }
        let mut domains: Vec<&str> = self
            .rules()
            .iter()
            .map(|rule| {
                let host = rule.host.split(':').next().unwrap_or_default();
                host.strip_prefix("*.").unwrap_or(host)
            })
            .filter(|host| !host.is_empty())
            .collect();
        domains.dedup();
        let terms: Vec<String> = domains
            .into_iter()
            .take(MAX_SITE_TERMS)
            .map(|d| format!("site:{d}"))
            .collect();
        Ok(format!("{query} ({})", terms.join(" OR ")))
    }

    fn allows_url(&self, url: &str) -> bool {
        reqwest::Url::parse(url).is_ok_and(|url| self.check_url(&url).is_ok())
    }

    /// Whether `url` is on an allowed site, by the egress rules (host,
    /// `*.domain`, port), and not one of this computer's own addresses
    fn check_url(&self, url: &reqwest::Url) -> Result<(), String> {
        if self.rules().is_empty() {
            return Err(no_sites());
        }
        if !matches!(url.scheme(), "http" | "https") {
            return Err("Only http and https pages can be read".to_string());
        }
        let host = url.host_str().unwrap_or_default();
        if is_local(url) {
            return Err(format!(
                "{host} is on this computer or its network, which agents can't read"
            ));
        }
        // `None` for the scheme's default port, as the rules expect
        if self.rules.allows_request(host, url.port(), "GET") {
            Ok(())
        } else {
            Err(format!(
                "{host} isn't one of this agent's allowed sites. Add it to the agent's Internet access to search and read it"
            ))
        }
    }
}

fn no_sites() -> String {
    "This agent has no allowed sites. Add the sites it may search to its Internet access settings"
        .to_string()
}

/// Loopback, private, link-local, and `localhost` addresses: the host's
/// own services must never be reachable through `read_page`
fn is_local(url: &reqwest::Url) -> bool {
    use std::net::IpAddr;
    let Some(host) = url.host_str() else {
        return true;
    };
    // IPv6 literals come bracketed
    let Ok(ip) = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<IpAddr>()
    else {
        let name = host.trim_end_matches('.').to_ascii_lowercase();
        return name == "localhost" || name.ends_with(".localhost") || name.ends_with(".local");
    };
    match ip {
        IpAddr::V4(ip) => {
            ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || (ip.segments()[0] & 0xfe00) == 0xfc00
                || (ip.segments()[0] & 0xffc0) == 0xfe80
                || ip
                    .to_ipv4_mapped()
                    .is_some_and(|v4| v4.is_loopback() || v4.is_private())
        }
    }
}

fn string_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("Missing string argument '{key}'"))
}

/// `&amp;` and friends, and numeric references
fn decode_entities(text: &str) -> String {
    static NUMERIC: OnceLock<Regex> = OnceLock::new();
    let numeric = NUMERIC.get_or_init(|| regex(r"&#(x[0-9a-fA-F]+|[0-9]+);"));
    let text = numeric.replace_all(text, |caps: &regex::Captures| {
        let code = &caps[1];
        let value = match code.strip_prefix('x') {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => code.parse().ok(),
        };
        value
            .and_then(char::from_u32)
            .map_or_else(String::new, |c| c.to_string())
    });
    text.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Tags out, entities decoded, whitespace collapsed
fn strip_tags(html: &str) -> String {
    static TAG: OnceLock<Regex> = OnceLock::new();
    static SPACE: OnceLock<Regex> = OnceLock::new();
    let text = TAG.get_or_init(|| regex(r"<[^>]*>")).replace_all(html, " ");
    let text = decode_entities(&text);
    SPACE
        .get_or_init(|| regex(r"\s+"))
        .replace_all(&text, " ")
        .trim()
        .to_string()
}

/// Bing's RSS results: `<item><title/><link/><description/></item>`
fn parse_bing_rss(xml: &str) -> Vec<Hit> {
    static ITEM: OnceLock<Regex> = OnceLock::new();
    let item = ITEM.get_or_init(|| regex(r"(?s)<item>(.*?)</item>"));
    let field = |body: &str, name: &str| {
        regex(&format!(r"(?s)<{name}>(.*?)</{name}>"))
            .captures(body)
            .map(|c| {
                let value = c[1].trim();
                let value = value
                    .strip_prefix("<![CDATA[")
                    .and_then(|v| v.strip_suffix("]]>"))
                    .unwrap_or(value);
                strip_tags(&decode_entities(value))
            })
            .unwrap_or_default()
    };
    item.captures_iter(xml)
        .map(|c| Hit {
            title: field(&c[1], "title"),
            url: field(&c[1], "link"),
            description: field(&c[1], "description"),
        })
        .filter(|hit| !hit.url.is_empty())
        .collect()
}

/// DuckDuckGo's HTML results: `result__a` links (through its `/l/?uddg=`
/// redirect) and `result__snippet`s
fn parse_duckduckgo(html: &str) -> Vec<Hit> {
    static LINK: OnceLock<Regex> = OnceLock::new();
    static SNIPPET: OnceLock<Regex> = OnceLock::new();
    let link = LINK.get_or_init(|| {
        regex(r#"(?s)<a[^>]*class="[^"]*result__a[^"]*"[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#)
    });
    let snippet = SNIPPET.get_or_init(|| {
        regex(r#"(?s)class="[^"]*result__snippet[^"]*"[^>]*>(.*?)</(?:a|td|div)>"#)
    });
    let snippets: Vec<String> = snippet
        .captures_iter(html)
        .map(|c| strip_tags(&c[1]))
        .collect();
    link.captures_iter(html)
        .enumerate()
        .filter_map(|(i, c)| {
            let href = decode_entities(&c[1]);
            let absolute = if href.starts_with("//") {
                format!("https:{href}")
            } else {
                href
            };
            let parsed = reqwest::Url::parse(&absolute).ok()?;
            let url = parsed
                .query_pairs()
                .find(|(k, _)| k == "uddg")
                .map_or_else(|| absolute.clone(), |(_, v)| v.into_owned());
            Some(Hit {
                title: strip_tags(&c[2]),
                url,
                description: snippets.get(i).cloned().unwrap_or_default(),
            })
        })
        .collect()
}

fn page_title(html: &str) -> String {
    static TITLE: OnceLock<Regex> = OnceLock::new();
    TITLE
        .get_or_init(|| regex(r"(?is)<title[^>]*>(.*?)</title>"))
        .captures(html)
        .map(|c| strip_tags(&c[1]))
        .unwrap_or_default()
}

/// A page's readable text: no scripts, styles, or markup
fn html_text(html: &str) -> String {
    static HIDDEN: OnceLock<Regex> = OnceLock::new();
    static BREAK: OnceLock<Regex> = OnceLock::new();
    static TAG: OnceLock<Regex> = OnceLock::new();
    static BLANK: OnceLock<Regex> = OnceLock::new();
    let html = HIDDEN
        .get_or_init(|| {
            regex(r"(?is)<(script|style|noscript|svg|head|template)\b[^>]*>.*?</(script|style|noscript|svg|head|template)>|<!--.*?-->")
        })
        .replace_all(html, " ");
    // Block ends become line breaks, so paragraphs stay apart
    let html = BREAK
        .get_or_init(|| regex(r"(?i)<(br|/p|/div|/li|/h[1-6]|/tr|/section|/article)\b[^>]*>"))
        .replace_all(&html, "\n");
    let text = TAG
        .get_or_init(|| regex(r"<[^>]*>"))
        .replace_all(&html, " ");
    let text = decode_entities(&text);
    let lines: Vec<String> = text
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    BLANK
        .get_or_init(|| regex(r"\n{3,}"))
        .replace_all(&lines.join("\n"), "\n\n")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Query;
    use axum::routing::get;
    use axum::Router;
    use std::collections::HashMap;

    fn sites(hosts: &[&str]) -> serde_yaml::Value {
        let rules: Vec<HostRule> = hosts
            .iter()
            .map(|host| HostRule {
                host: host.to_string(),
                methods: Vec::new(),
            })
            .collect();
        let mut options = serde_yaml::Mapping::new();
        options.insert(ALLOWED_SITES.into(), serde_yaml::to_value(rules).unwrap());
        serde_yaml::Value::Mapping(options)
    }

    async fn serve(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}/search")
    }

    const RSS: &str = r#"<?xml version="1.0"?><rss><channel><title>Bing</title>
<item><title>Rust &amp; MCP</title><link>https://docs.rs/rmcp</link><description>The &lt;b&gt;MCP&lt;/b&gt; SDK</description></item>
<item><title>Elsewhere</title><link>https://evil.example/x</link><description>no</description></item>
<item><title>Blog</title><link>https://blog.rust-lang.org/post</link><description>News</description></item>
</channel></rss>"#;

    #[tokio::test]
    async fn searches_only_the_agent_s_sites() {
        let seen = Arc::new(std::sync::Mutex::new(String::new()));
        let query = seen.clone();
        let router = Router::new().route(
            "/search",
            get(move |Query(q): Query<HashMap<String, String>>| {
                let query = query.clone();
                async move {
                    *query.lock().unwrap() = format!("{} {}", q["q"], q["format"]);
                    RSS
                }
            }),
        );
        let search = WebSearchConnector::new(Engine::Bing, Some(serve(router).await));
        let options = sites(&["docs.rs", "*.rust-lang.org"]);
        let value = search
            .call_tool_for("search", &json!({"query": "rust mcp"}), Some(&options))
            .await
            .unwrap();
        assert_eq!(
            *seen.lock().unwrap(),
            "rust mcp (site:docs.rs OR site:rust-lang.org) rss"
        );
        let results = value["results"].as_array().unwrap();
        let urls: Vec<&str> = results.iter().map(|r| r["url"].as_str().unwrap()).collect();
        assert_eq!(
            urls,
            ["https://docs.rs/rmcp", "https://blog.rust-lang.org/post"]
        );
        assert_eq!(results[0]["title"], "Rust & MCP");
        assert_eq!(results[0]["description"], "The MCP SDK");

        // Every site: the query as asked, every result
        let value = search
            .call_tool_for(
                "search",
                &json!({"query": "x", "count": 2}),
                Some(&sites(&["*"])),
            )
            .await
            .unwrap();
        assert_eq!(value["results"].as_array().unwrap().len(), 2);
        assert!(seen.lock().unwrap().starts_with("x rss"));
    }

    #[tokio::test]
    async fn an_agent_without_sites_cannot_search_or_read() {
        let search = WebSearchConnector::new(Engine::Bing, Some("http://127.0.0.1:9/".into()));
        for (tool, args) in [
            ("search", json!({"query": "x"})),
            ("read_page", json!({"url": "https://docs.rs"})),
        ] {
            let error = search.call_tool(tool, &args).await.unwrap_err();
            assert!(error.contains("no allowed sites"), "{tool}: {error}");
        }
    }

    async fn read(url: &str, hosts: &[&str]) -> Result<Value, String> {
        WebSearchConnector::new(Engine::Bing, None)
            .call_tool_for("read_page", &json!({ "url": url }), Some(&sites(hosts)))
            .await
    }

    #[tokio::test]
    async fn reads_pages_only_on_allowed_public_sites() {
        let error = read("https://evil.example/", &["docs.rs"])
            .await
            .unwrap_err();
        assert!(
            error.contains("isn't one of this agent's allowed sites"),
            "{error}"
        );
        for local in [
            "http://127.0.0.1:11435/api",
            "http://localhost/",
            "http://192.168.1.10/",
            "http://[::1]/",
        ] {
            let error = read(local, &["*"]).await.unwrap_err();
            assert!(error.contains("on this computer"), "{local}: {error}");
        }
        let error = read("file:///etc/passwd", &["*"]).await.unwrap_err();
        assert!(error.contains("http"), "{error}");
    }

    #[test]
    fn parses_duckduckgo_results() {
        let html = r#"<div class="result"><a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdocs.rs%2Frmcp&amp;rut=abc">Rust <b>MCP</b></a>
<a class="result__snippet" href="x">The SDK &amp; more</a></div>"#;
        assert_eq!(
            parse_duckduckgo(html),
            [Hit {
                title: "Rust MCP".into(),
                url: "https://docs.rs/rmcp".into(),
                description: "The SDK & more".into(),
            }]
        );
    }

    #[test]
    fn pages_read_as_text() {
        let html = "<html><head><title>Docs &amp; more</title><style>p{}</style></head><body><script>alert(1)</script><h1>Intro</h1><p>One&nbsp;two</p><!-- hidden --><p>Three</p></body></html>";
        assert_eq!(page_title(html), "Docs & more");
        assert_eq!(html_text(html), "Intro\nOne two\nThree");
    }

    #[test]
    fn picks_the_engine() {
        assert_eq!(Engine::parse(None).unwrap(), Engine::Bing);
        assert_eq!(
            Engine::parse(Some("DuckDuckGo")).unwrap(),
            Engine::DuckDuckGo
        );
        assert!(Engine::parse(Some("google")).is_err());
    }
}
