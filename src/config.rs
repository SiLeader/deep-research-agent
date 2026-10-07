use anyhow::Context;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    pub models: Vec<ModelConfig>,
    #[serde(default)]
    pub agent: AgentConfig,
    pub providers: Vec<ProviderConfig>,
    pub tools: ToolsConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ServerConfig {
    pub host: String,
    pub port: u16,
    /// Environment variable holding the bearer token required on `/api/` routes.
    pub api_key_env: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct AgentConfig {
    pub model: Option<String>,
    pub max_llm_calls: usize,
    pub max_research_loops: usize,
    pub max_tool_context_chars: usize,
    pub max_total_llm_calls: usize,
    pub research_timeout_secs: u64,
    pub planner: AgentRoleConfig,
    pub research: AgentRoleConfig,
    pub gap_judger: AgentRoleConfig,
    pub explorer: AgentRoleConfig,
    pub synthesizer: AgentRoleConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentRoleConfig {
    pub model: Option<String>,
    pub system_prompt: Option<String>,
}

pub(crate) struct MustAgentConfig {
    pub max_llm_calls: usize,
    pub max_tool_context_chars: usize,
    pub limits: deep_research_orchestrator::ResearchLimits,
    pub planner: MustAgentRoleConfig,
    pub research: MustAgentRoleConfig,
    pub gap_judger: MustAgentRoleConfig,
    pub explorer: MustAgentRoleConfig,
    pub synthesizer: MustAgentRoleConfig,
}

pub(crate) struct MustAgentRoleConfig {
    pub model: String,
    pub system_prompt: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelConfig {
    pub id: String,
    pub provider: String,
    pub name: String,
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: usize,
    #[serde(default = "default_llm_request_timeout")]
    pub request_timeout_secs: u64,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
}

fn default_max_retries() -> u32 {
    3
}

fn default_llm_request_timeout() -> u64 {
    120
}

fn default_max_concurrency() -> usize {
    1
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolsConfig {
    pub web_search: WebSearchConfig,
    #[serde(default)]
    pub fetched: FetchedSettings,
    #[serde(default)]
    pub web_fetch: deep_research_tools::tools::WebRequestLimits,
}

// Fields are listed explicitly rather than flattened so unknown keys are rejected.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct FetchedSettings {
    pub chunk_size: usize,
    pub default_top_k: usize,
    pub max_top_k: usize,
    pub embedding: Option<EmbeddingConfig>,
    pub reranker: Option<RerankerConfig>,
}

impl Default for FetchedSettings {
    fn default() -> Self {
        let search = deep_research_tools::tools::search_fetched::FetchedConfig::default();
        Self {
            chunk_size: search.chunk_size,
            default_top_k: search.default_top_k,
            max_top_k: search.max_top_k,
            embedding: None,
            reranker: None,
        }
    }
}

impl FetchedSettings {
    pub fn search(&self) -> deep_research_tools::tools::search_fetched::FetchedConfig {
        deep_research_tools::tools::search_fetched::FetchedConfig {
            chunk_size: self.chunk_size,
            default_top_k: self.default_top_k,
            max_top_k: self.max_top_k,
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        self.search().validate()?;
        if let Some(embedding) = &self.embedding {
            anyhow::ensure!(
                embedding.request_timeout_secs > 0,
                "tools.fetched.embedding.request_timeout_secs must be positive"
            );
            anyhow::ensure!(
                embedding.batch_size > 0,
                "tools.fetched.embedding.batch_size must be positive"
            );
            anyhow::ensure!(
                !embedding.model.trim().is_empty(),
                "tools.fetched.embedding.model must not be empty"
            );
        }
        if let Some(reranker) = &self.reranker {
            anyhow::ensure!(
                !reranker.model.trim().is_empty(),
                "tools.fetched.reranker.model must not be empty"
            );
            anyhow::ensure!(
                !reranker.endpoint.trim().is_empty(),
                "tools.fetched.reranker.endpoint must not be empty"
            );
            anyhow::ensure!(
                reranker
                    .api_key_env
                    .as_ref()
                    .is_none_or(|name| !name.trim().is_empty()),
                "tools.fetched.reranker.api_key_env must not be empty"
            );
            anyhow::ensure!(
                reranker.request_timeout_secs > 0,
                "tools.fetched.reranker.request_timeout_secs must be positive"
            );
            anyhow::ensure!(
                (1..=tokio::sync::Semaphore::MAX_PERMITS).contains(&reranker.max_concurrency),
                "invalid tools.fetched.reranker.max_concurrency"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EmbeddingConfig {
    /// ID in [[models]], using an OpenAI-compatible embeddings provider.
    pub model: String,
    #[serde(default = "default_retrieval_timeout")]
    pub request_timeout_secs: u64,
    /// Maximum inputs per embedding request.
    #[serde(default = "default_embedding_batch_size")]
    pub batch_size: usize,
}

fn default_embedding_batch_size() -> usize {
    deep_research_tools::fetched::DEFAULT_EMBEDDING_BATCH_SIZE
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RerankerConfig {
    /// Model name sent to the Cohere-compatible /rerank endpoint.
    pub model: String,
    pub endpoint: String,
    pub api_key_env: Option<String>,
    #[serde(default = "default_retrieval_timeout")]
    pub request_timeout_secs: u64,
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: usize,
}

fn default_retrieval_timeout() -> u64 {
    60
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WebSearchConfig {
    pub searxng: SearxngConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearxngConfig {
    pub endpoint: String,
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_secs: u64,
    #[serde(default = "default_web_request_timeout")]
    pub request_timeout_secs: u64,
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: usize,
}

impl SearxngConfig {
    pub fn limits(&self) -> deep_research_tools::tools::WebRequestLimits {
        deep_research_tools::tools::WebRequestLimits {
            connect_timeout_secs: self.connect_timeout_secs,
            request_timeout_secs: self.request_timeout_secs,
            max_body_bytes: self.max_body_bytes,
        }
    }
}

fn default_connect_timeout() -> u64 {
    deep_research_tools::tools::WebRequestLimits::default().connect_timeout_secs
}

fn default_web_request_timeout() -> u64 {
    deep_research_tools::tools::WebRequestLimits::default().request_timeout_secs
}

fn default_max_body_bytes() -> usize {
    deep_research_tools::tools::WebRequestLimits::default().max_body_bytes
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderConfig {
    pub id: String,
    #[serde(rename = "type")]
    pub provider_type: ProviderType,
    pub api_key_env: String,
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) enum ProviderType {
    OpenAI,
    Anthropic,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model: None,
            max_llm_calls: 30,
            max_research_loops: 10,
            max_tool_context_chars: 200_000,
            max_total_llm_calls: 2_000,
            research_timeout_secs: 3_600,
            planner: Default::default(),
            research: Default::default(),
            gap_judger: Default::default(),
            explorer: Default::default(),
            synthesizer: Default::default(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            api_key_env: None,
        }
    }
}

impl AgentConfig {
    pub fn into_must(self) -> anyhow::Result<MustAgentConfig> {
        anyhow::ensure!(
            self.max_llm_calls > 0,
            "agent.max_llm_calls must be positive"
        );
        anyhow::ensure!(
            self.max_tool_context_chars > 0,
            "agent.max_tool_context_chars must be positive"
        );
        let limits = deep_research_orchestrator::ResearchLimits {
            max_research_loops: self.max_research_loops,
            timeout: Some(std::time::Duration::from_secs(self.research_timeout_secs)),
            max_total_llm_calls: Some(self.max_total_llm_calls),
        };
        limits.validate().context(
            "agent.max_research_loops, agent.research_timeout_secs and agent.max_total_llm_calls must be positive",
        )?;
        let planner_model = self
            .planner
            .model
            .or_else(|| self.model.clone())
            .context("`agent.planner.model` or `agent.model` is not set in config")?;
        let research_model = self
            .research
            .model
            .or_else(|| self.model.clone())
            .context("`agent.research.model` or `agent.model` is not set in config")?;
        let gap_judger_model = self
            .gap_judger
            .model
            .or_else(|| self.model.clone())
            .unwrap_or_else(|| research_model.clone());
        let explorer_model = self
            .explorer
            .model
            .or_else(|| self.model.clone())
            .unwrap_or_else(|| research_model.clone());
        let synthesizer_model = self
            .synthesizer
            .model
            .or_else(|| self.model.clone())
            .context("`agent.synthesizer.model` or `agent.model` is not set in config")?;
        Ok(MustAgentConfig {
            max_llm_calls: self.max_llm_calls,
            max_tool_context_chars: self.max_tool_context_chars,
            limits,
            planner: MustAgentRoleConfig {
                model: planner_model,
                system_prompt: self
                    .planner
                    .system_prompt
                    .unwrap_or_else(|| include_str!("assets/planner.md").to_string()),
            },
            research: MustAgentRoleConfig {
                model: research_model,
                system_prompt: self
                    .research
                    .system_prompt
                    .unwrap_or_else(|| include_str!("assets/research.md").to_string()),
            },
            gap_judger: MustAgentRoleConfig {
                model: gap_judger_model,
                system_prompt: self
                    .gap_judger
                    .system_prompt
                    .unwrap_or_else(|| include_str!("assets/gap_judger.md").to_string()),
            },
            explorer: MustAgentRoleConfig {
                model: explorer_model,
                system_prompt: self
                    .explorer
                    .system_prompt
                    .unwrap_or_else(|| include_str!("assets/explorer.md").to_string()),
            },
            synthesizer: MustAgentRoleConfig {
                model: synthesizer_model,
                system_prompt: self
                    .synthesizer
                    .system_prompt
                    .unwrap_or_else(|| include_str!("assets/synthesizer.md").to_string()),
            },
        })
    }
}

impl Config {
    pub fn from_file(path: &str) -> anyhow::Result<Self> {
        let config_str = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&config_str)?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_partial_server_settings() {
        let text = include_str!("../config.example.toml")
            .replace("host = \"127.0.0.1\"\n", "")
            .replace("max_concurrency = 1\n", "");
        let config: Config = toml::from_str(&text).unwrap();
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.models[0].max_concurrency, 1);
        let agent = config.agent.into_must().unwrap();
        assert_eq!(agent.planner.model, "default");
        assert_eq!(agent.research.model, "default");
        assert_eq!(agent.gap_judger.model, "default");
        assert_eq!(agent.explorer.model, "default");
        assert_eq!(agent.synthesizer.model, "default");
        assert_eq!(
            agent.explorer.system_prompt,
            include_str!("assets/explorer.md")
        );
    }

    #[test]
    fn role_overrides_and_research_fallbacks() {
        let agent: AgentConfig = toml::from_str(
            r#"
            [planner]
            model = "planner"
            system_prompt = "custom planner"
            [research]
            model = "research"
            system_prompt = "custom research"
            [gap_judger]
            system_prompt = "custom gap"
            [explorer]
            system_prompt = "custom explorer"
            [synthesizer]
            model = "synthesizer"
            system_prompt = "custom synthesizer"
        "#,
        )
        .unwrap();
        let agent = agent.into_must().unwrap();
        assert_eq!(agent.gap_judger.model, "research");
        assert_eq!(agent.explorer.model, "research");
        assert_eq!(agent.planner.system_prompt, "custom planner");
        assert_eq!(agent.research.system_prompt, "custom research");
        assert_eq!(agent.gap_judger.system_prompt, "custom gap");
        assert_eq!(agent.explorer.system_prompt, "custom explorer");
        assert_eq!(agent.synthesizer.system_prompt, "custom synthesizer");
        assert!(AgentConfig::default().into_must().is_err());
    }

    #[test]
    fn explicit_role_models_override_shared_model() {
        let config: AgentConfig = toml::from_str(
            r#"
            model = "shared"
            [planner]
            model = "planner"
            [research]
            model = "research"
            [gap_judger]
            model = "gap"
            [explorer]
            model = "explorer"
            [synthesizer]
            model = "synthesizer"
            "#,
        )
        .unwrap();
        let agent = config.into_must().unwrap();
        assert_eq!(agent.planner.model, "planner");
        assert_eq!(agent.research.model, "research");
        assert_eq!(agent.gap_judger.model, "gap");
        assert_eq!(agent.explorer.model, "explorer");
        assert_eq!(agent.synthesizer.model, "synthesizer");
    }

    #[test]
    fn shared_model_takes_precedence_over_research_fallback() {
        let config: AgentConfig = toml::from_str(
            r#"
            model = "shared"
            [research]
            model = "research"
            "#,
        )
        .unwrap();
        let agent = config.into_must().unwrap();
        assert_eq!(agent.research.model, "research");
        assert_eq!(agent.gap_judger.model, "shared");
        assert_eq!(agent.explorer.model, "shared");
    }

    #[test]
    fn missing_required_models_identify_the_role() {
        for missing in ["planner", "research", "synthesizer"] {
            let mut config = AgentConfig::default();
            if missing != "planner" {
                config.planner.model = Some("planner".into());
            }
            if missing != "research" {
                config.research.model = Some("research".into());
            }
            if missing != "synthesizer" {
                config.synthesizer.model = Some("synthesizer".into());
            }
            let error = config.into_must().err().unwrap();
            assert_eq!(
                error.to_string(),
                format!("`agent.{missing}.model` or `agent.model` is not set in config")
            );
        }
    }

    #[test]
    fn omitted_prompts_use_embedded_defaults() {
        let agent = AgentConfig {
            model: Some("shared".into()),
            ..Default::default()
        }
        .into_must()
        .unwrap();
        assert_eq!(
            agent.planner.system_prompt,
            include_str!("assets/planner.md")
        );
        assert_eq!(
            agent.research.system_prompt,
            include_str!("assets/research.md")
        );
        assert_eq!(
            agent.gap_judger.system_prompt,
            include_str!("assets/gap_judger.md")
        );
        assert_eq!(
            agent.explorer.system_prompt,
            include_str!("assets/explorer.md")
        );
        assert_eq!(
            agent.synthesizer.system_prompt,
            include_str!("assets/synthesizer.md")
        );
    }

    #[test]
    fn omitted_server_and_agent_sections_use_defaults() {
        let config: Config = toml::from_str(
            r#"
            models = []
            providers = []
            [tools.web_search.searxng]
            endpoint = "http://localhost/search"
        "#,
        )
        .unwrap();
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 8080);
        assert!(config.agent.model.is_none());
        assert!(config.agent.into_must().is_err());
    }

    #[test]
    fn fetched_settings_defaults_overrides_and_validation() {
        use deep_research_tools::tools::search_fetched::FetchedConfig;
        let defaults: FetchedConfig = toml::from_str("").unwrap();
        assert_eq!(defaults.chunk_size, 1024);
        assert_eq!(defaults.default_top_k, 5);
        assert_eq!(defaults.max_top_k, 20);
        defaults.validate().unwrap();
        let custom: FetchedConfig =
            toml::from_str("chunk_size = 128\ndefault_top_k = 2\nmax_top_k = 4").unwrap();
        assert_eq!(custom.chunk_size, 128);
        assert_eq!(custom.default_top_k, 2);
        assert_eq!(custom.max_top_k, 4);
        custom.validate().unwrap();
        for text in [
            "chunk_size = 0",
            "default_top_k = 0",
            "max_top_k = 0",
            "default_top_k = 21",
        ] {
            assert!(
                toml::from_str::<FetchedConfig>(text)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
        let config: Config = toml::from_str(include_str!("../config.example.toml")).unwrap();
        config.tools.fetched.validate().unwrap();
    }

    #[test]
    fn execution_limits_default_override_and_reject_zero() {
        let config: Config = toml::from_str(include_str!("../config.example.toml")).unwrap();
        assert_eq!(config.agent.max_llm_calls, 30);
        assert_eq!(config.models[0].request_timeout_secs, 120);
        assert_eq!(config.tools.web_fetch.max_body_bytes, 2 * 1024 * 1024);
        assert_eq!(
            config
                .tools
                .web_search
                .searxng
                .limits()
                .request_timeout_secs,
            60
        );
        let mut agent = AgentConfig {
            model: Some("default".into()),
            max_llm_calls: 2,
            ..Default::default()
        };
        assert_eq!(agent.clone().into_must().unwrap().max_llm_calls, 2);
        agent.max_llm_calls = 0;
        assert!(agent.into_must().is_err());
        let limits: deep_research_tools::tools::WebRequestLimits = toml::from_str(
            "connect_timeout_secs = 2\nrequest_timeout_secs = 3\nmax_body_bytes = 4",
        )
        .unwrap();
        assert_eq!(limits.connect_timeout_secs, 2);
        assert_eq!(limits.request_timeout_secs, 3);
        assert_eq!(limits.max_body_bytes, 4);
        for text in [
            "connect_timeout_secs = 0",
            "request_timeout_secs = 0",
            "max_body_bytes = 0",
        ] {
            let limits: deep_research_tools::tools::WebRequestLimits =
                toml::from_str(text).unwrap();
            assert!(limits.validate().is_err());
        }
    }

    #[test]
    fn unknown_keys_are_rejected_at_every_level() {
        let base = include_str!("../config.example.toml");
        toml::from_str::<Config>(base).unwrap();
        for (section, typo) in [
            ("[server]\n", "api_key = 'x'\n"),
            ("[agent]\n", "max_llm_call = 3\n"),
            ("[agent.planner]\n", "prompt = 'x'\n"),
            ("[[models]]\n", "timeout = 3\n"),
            ("[[providers]]\n", "key = 'x'\n"),
            ("[tools.web_search.searxng]\n", "timeout = 3\n"),
            ("[tools.web_fetch]\n", "timeout = 3\n"),
            ("[tools.fetched]\n", "top_k = 3\n"),
        ] {
            let text = base.replacen(section, &format!("{section}{typo}"), 1);
            assert_ne!(text, base, "{section}");
            assert!(toml::from_str::<Config>(&text).is_err(), "{section}{typo}");
        }
    }

    #[test]
    fn research_limits_default_and_reject_zero() {
        let config: Config = toml::from_str(include_str!("../config.example.toml")).unwrap();
        assert_eq!(config.models[0].max_retries, 3);
        assert!(config.server.api_key_env.is_none());
        let agent = config.agent.into_must().unwrap();
        assert_eq!(agent.max_tool_context_chars, 200_000);
        assert_eq!(agent.limits.max_research_loops, 10);
        assert_eq!(agent.limits.max_total_llm_calls, Some(2_000));
        assert_eq!(
            agent.limits.timeout,
            Some(std::time::Duration::from_secs(3_600))
        );
        for text in [
            "max_research_loops = 0",
            "max_tool_context_chars = 0",
            "max_total_llm_calls = 0",
            "research_timeout_secs = 0",
        ] {
            let agent: AgentConfig = toml::from_str(&format!("model = 'm'\n{text}")).unwrap();
            assert!(agent.into_must().is_err(), "{text}");
        }
    }

    #[test]
    fn model_request_timeout_defaults_overrides_and_rejects_zero() {
        let base = "id = 'test'\nprovider = 'fixture'\nname = 'test-model'\n";
        let default: ModelConfig = toml::from_str(base).unwrap();
        assert_eq!(default.request_timeout_secs, 120);
        let custom: ModelConfig =
            toml::from_str(&format!("{base}request_timeout_secs = 5")).unwrap();
        assert_eq!(custom.request_timeout_secs, 5);
        let invalid: ModelConfig =
            toml::from_str(&format!("{base}request_timeout_secs = 0")).unwrap();
        let error = super::super::build_model_services(vec![invalid], vec![])
            .err()
            .unwrap();
        assert!(
            error
                .to_string()
                .contains("Invalid request_timeout_secs for model: test")
        );
    }
}
