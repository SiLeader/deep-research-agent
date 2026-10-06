use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    pub models: Vec<ModelConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ServerConfig {
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ModelConfig {
    pub provider: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ProviderConfig {
    pub name: String,
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

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
        }
    }
}

impl Config {
    pub fn from_file(path: &str) -> anyhow::Result<Self> {
        let config_str = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&config_str)?;
        Ok(config)
    }
}
