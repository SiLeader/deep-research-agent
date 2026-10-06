use anyhow::Context;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
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
#[serde(default)]
pub(crate) struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub(crate) struct AgentConfig {
    pub model: Option<String>,
    #[serde(default = "default_max_llm_calls")]
    pub max_llm_calls: usize,
    pub planner: AgentRoleConfig,
    pub research: AgentRoleConfig,
    pub gap_judger: AgentRoleConfig,
    pub explorer: AgentRoleConfig,
    pub synthesizer: AgentRoleConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct AgentRoleConfig {
    pub model: Option<String>,
    pub system_prompt: Option<String>,
}

pub(crate) struct MustAgentConfig {
    pub max_llm_calls: usize,
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
pub(crate) struct ModelConfig {
    pub id: String,
    pub provider: String,
    pub name: String,
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: usize,
}

fn default_max_concurrency() -> usize {
    1
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ToolsConfig {
    pub web_search: WebSearchConfig,
    #[serde(default)]
    pub web_fetch: deep_research_tools::tools::WebRequestLimits,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct WebSearchConfig {
    pub searxng: SearxngConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SearxngConfig {
    pub endpoint: String,
    #[serde(flatten)]
    pub limits: deep_research_tools::tools::WebRequestLimits,
}

#[derive(Debug, Clone, Deserialize)]
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

fn default_max_llm_calls() -> usize {
    30
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model: None,
            max_llm_calls: default_max_llm_calls(),
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
        }
    }
}

impl AgentConfig {
    pub fn into_must(self) -> anyhow::Result<MustAgentConfig> {
        anyhow::ensure!(
            self.max_llm_calls > 0,
            "agent.max_llm_calls must be positive"
        );
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
    fn execution_limits_default_override_and_reject_zero() {
        let config: Config = toml::from_str(include_str!("../config.example.toml")).unwrap();
        assert_eq!(config.agent.max_llm_calls, 30);
        assert_eq!(config.tools.web_fetch.max_body_bytes, 2 * 1024 * 1024);
        assert_eq!(
            config.tools.web_search.searxng.limits.request_timeout_secs,
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
}
