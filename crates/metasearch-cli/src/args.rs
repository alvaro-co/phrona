use std::io;

use clap::{Args, CommandFactory, Parser, Subcommand};

use metasearch::{Category, Profile, SearchOptions, TimeRange};

#[derive(Parser)]
#[command(
    name = "ms",
    version,
    about = "MetasearchRS command-line interface",
    long_about = "Search 25 engines across 5 categories, get suggestions, extract pages, \
                  produce AI-grounded answers, and probe engine availability. \
                  All commands accept --json for machine-readable output."
)]
pub struct Cli {
    /// Machine-readable JSON output
    #[arg(long, global = true)]
    pub json: bool,

    /// Browser impersonation profile
    #[arg(long, global = true, value_parser = profile_parser, default_value = "chrome")]
    pub profile: Profile,

    /// Proxy URL (e.g. socks5://127.0.0.1:9050), repeatable
    #[arg(long, global = true)]
    pub proxy: Vec<String>,

    /// Request timeout in seconds
    #[arg(long, global = true, default_value_t = 20)]
    pub timeout: u64,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Search engines and print merged, ranked results
    Search(SearchArgs),
    /// Query autocomplete sources
    Suggest(SuggestArgs),
    /// Extract readable text from a web page
    Extract(ExtractArgs),
    /// Grounded search: synthesized answer plus ranked sources (RAG)
    Ground(GroundArgs),
    /// List engines per category
    Engines(EnginesArgs),
    /// Probe engine availability across every category
    Test(TestArgs),
    /// Start the full server: REST API plus MCP-over-TCP
    Serve(ServeArgs),
    /// Serve MCP over stdio only (for MCP clients)
    Mcp,
    /// Generate shell completion script
    Completions(CompletionsArgs),
}

#[derive(Args)]
pub struct SearchArgs {
    /// Search query
    pub query: String,

    /// Result category: web | images | news | videos | books
    #[arg(long, value_parser = category_parser, default_value = "web")]
    pub category: Category,

    /// Comma-separated engine names (default: all of the category)
    #[arg(long)]
    pub engines: Option<String>,

    /// Maximum merged results
    #[arg(long, default_value_t = 15)]
    pub max_results: usize,

    /// SafeSearch level: off | moderate | strict
    #[arg(long, value_parser = safesearch_parser, default_value = "moderate")]
    pub safesearch: metasearch::SafeSearch,

    /// Region (e.g. us-en, de-de)
    #[arg(long)]
    pub region: Option<String>,

    /// Language (e.g. en)
    #[arg(long)]
    pub language: Option<String>,

    /// Time range: day | week | month | year
    #[arg(long, value_parser = time_range_parser)]
    pub time_range: Option<TimeRange>,

    /// Engine filter string (e.g. site:github.com)
    #[arg(long)]
    pub filters: Option<String>,

    /// Result page
    #[arg(long, default_value_t = 1)]
    pub page: u32,
}

#[derive(Args)]
pub struct SuggestArgs {
    /// Query prefix
    pub query: String,

    /// Comma-separated sources (default: all)
    #[arg(long)]
    pub source: Option<String>,

    /// Region (e.g. us-en)
    #[arg(long, default_value = "us-en")]
    pub region: String,
}

#[derive(Args)]
pub struct ExtractArgs {
    /// Page URL
    pub url: String,

    /// Maximum characters of extracted text
    #[arg(long, default_value_t = 5000)]
    pub max_chars: usize,

    /// Bias the excerpt toward this query
    #[arg(long)]
    pub query: Option<String>,
}

#[derive(Args)]
pub struct GroundArgs {
    /// Search query
    pub query: String,

    /// Maximum sources to return
    #[arg(long, default_value_t = 8)]
    pub max_results: usize,

    /// Comma-separated engine names
    #[arg(long)]
    pub engines: Option<String>,
}

#[derive(Args)]
pub struct EnginesArgs {
    /// Filter by category
    #[arg(long, value_parser = category_parser)]
    pub category: Option<Category>,
}

#[derive(Args)]
pub struct TestArgs {
    /// Query used for the probe (default: "rust programming")
    #[arg(long, default_value = "rust programming")]
    pub query: String,

    /// Probe a single category only
    #[arg(long, value_parser = category_parser)]
    pub category: Option<Category>,
}

#[derive(Args)]
pub struct ServeArgs {
    /// REST API bind address (default: $META_ADDR or 127.0.0.1:8080)
    #[arg(long)]
    pub addr: Option<String>,

    /// MCP-over-TCP bind address (default: 127.0.0.1:8081)
    #[arg(long, default_value = "127.0.0.1:8081")]
    pub mcp_addr: String,

    /// API key required by clients (default: $META_API_KEY)
    #[arg(long)]
    pub api_key: Option<String>,

    /// Disable the MCP-over-TCP listener (REST only)
    #[arg(long)]
    pub no_mcp: bool,

    /// Disable the REST listener (MCP only)
    #[arg(long)]
    pub no_rest: bool,
}

#[derive(Args)]
pub struct CompletionsArgs {
    /// Shell to generate completions for
    #[arg(value_parser = ["bash", "zsh", "fish", "powershell", "elvish"])]
    pub shell: String,
}

pub fn category_parser(s: &str) -> Result<Category, String> {
    s.parse::<Category>().map_err(|_| {
        "invalid category, expected one of: web, images, news, videos, books".to_string()
    })
}

fn safesearch_parser(s: &str) -> Result<metasearch::SafeSearch, String> {
    s.parse::<metasearch::SafeSearch>()
        .map_err(|_| "invalid safesearch, expected one of: off, moderate, strict".to_string())
}

fn time_range_parser(s: &str) -> Result<TimeRange, String> {
    s.parse::<TimeRange>()
        .map_err(|_| "invalid time_range, expected one of: day, week, month, year".to_string())
}

fn profile_parser(s: &str) -> Result<Profile, String> {
    match s.to_ascii_lowercase().as_str() {
        "chrome" | "chrome148" => Ok(Profile::Chrome),
        "chrome149" => Ok(Profile::Chrome149),
        "chrome131" => Ok(Profile::Chrome131),
        "chrome120" => Ok(Profile::Chrome120),
        "chrome100" => Ok(Profile::Chrome100),
        "firefox" | "firefox148" => Ok(Profile::Firefox),
        "firefox139" => Ok(Profile::Firefox139),
        "safari" | "safari26" => Ok(Profile::Safari),
        "edge" | "edge148" => Ok(Profile::Edge),
        "opera" | "opera131" => Ok(Profile::Opera),
        "okhttp" => Ok(Profile::OkHttp),
        "random" => Ok(Profile::Random),
        _ => Err(format!(
            "unknown profile '{s}', expected chrome, firefox, safari, edge, opera, okhttp, random"
        )),
    }
}

impl Cli {
    pub fn base_options(&self, query: impl Into<String>) -> SearchOptions {
        let mut opts = SearchOptions::new(query);
        opts.profile = self.profile;
        opts.timeout = std::time::Duration::from_secs(self.timeout);
        opts.proxies = self.proxy.clone();
        opts
    }
}

pub fn print_completions(shell: &str) -> anyhow::Result<()> {
    let mut cmd = Cli::command();
    let shell = match shell {
        "bash" => clap_complete::Shell::Bash,
        "zsh" => clap_complete::Shell::Zsh,
        "fish" => clap_complete::Shell::Fish,
        "powershell" => clap_complete::Shell::PowerShell,
        "elvish" => clap_complete::Shell::Elvish,
        _ => unreachable!("validated by clap"),
    };
    clap_complete::generate(shell, &mut cmd, "ms", &mut io::stdout());
    Ok(())
}
