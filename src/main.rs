use anyhow::{Context, bail};
use clap::Parser;
use deep_research_agent_tools::ExplorerTool;
use deep_research_api::DeepResearchServer;
use deep_research_arbiter::semaphore::SemaphoreConcurrencyArbiter;
use deep_research_orchestrator::DeepResearchOrchestrator;
use deep_research_react_agent::ReActAgent;
use deep_research_runner::OneshotRunner;
use deep_research_tools::DeepResearchTools;
use deep_research_tools::tools::{web_fetch::WebFetchTool, web_search::WebSearchTool};
use genai::adapter::AdapterKind;
use genai::chat::{ChatOptions, ToolChoice};
use genai::resolver::{AuthData, Endpoint};
use genai::{Client, ModelIden, ServiceTarget};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

mod config;

#[derive(Debug, Parser)]
struct Args {
    #[arg(
        long,
        help = "Path to the configuration file",
        default_value = "/etc/deep-research-agent/config.toml"
    )]
    config: String,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let config = match config::Config::from_file(&args.config) {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };

    let agent_config = match config.agent.into_must() {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!("Failed to resolve agent configuration: {}", e);
            std::process::exit(1);
        }
    };

    if let Err(e) = run_server(
        config.server,
        config.models,
        config.providers,
        config.tools,
        agent_config,
    )
    .await
    {
        tracing::error!("Failed to run server: {:#}", e);
        std::process::exit(1);
    }
}

fn build_runners(
    models: Vec<config::ModelConfig>,
    providers: Vec<config::ProviderConfig>,
) -> anyhow::Result<HashMap<String, OneshotRunner>> {
    let mut provider_map = HashMap::new();
    for provider in providers {
        let id = provider.id.clone();
        if provider_map.insert(id.clone(), provider).is_some() {
            bail!("Duplicate provider ID: {id}");
        }
    }

    let mut limits = HashMap::new();
    for model in &models {
        if model.max_concurrency == 0 || model.max_concurrency > tokio::sync::Semaphore::MAX_PERMITS
        {
            bail!("Invalid max_concurrency for model: {}", model.id);
        }
        if limits
            .insert(model.id.clone(), model.max_concurrency)
            .is_some()
        {
            bail!("Duplicate model ID: {}", model.id);
        }
    }
    let arbiter = Arc::new(SemaphoreConcurrencyArbiter::new(limits));
    let mut runners = HashMap::new();
    for model in models {
        let provider = provider_map.get(&model.provider).with_context(|| {
            format!(
                "Unknown provider `{}` for model `{}`",
                model.provider, model.id
            )
        })?;
        let adapter = match provider.provider_type {
            config::ProviderType::OpenAI => AdapterKind::OpenAI,
            config::ProviderType::Anthropic => AdapterKind::Anthropic,
        };
        let api_key_env = provider.api_key_env.clone();
        let endpoint = provider.endpoint.clone();
        let name = model.name;
        let client = Client::builder()
            .with_adapter_kind(adapter)
            .with_model_mapper_fn(move |_| Ok(ModelIden::new(adapter, name.clone())))
            .with_auth_resolver_fn(move |_| Ok(Some(AuthData::from_env(api_key_env.clone()))))
            .with_service_target_resolver_fn(move |mut target: ServiceTarget| {
                if let Some(endpoint) = &endpoint {
                    target.endpoint = Endpoint::from_owned(endpoint.clone());
                }
                Ok(target)
            })
            .build();
        runners.insert(
            model.id.clone(),
            OneshotRunner::new(
                model.id,
                client,
                arbiter.clone(),
                ChatOptions::default().with_tool_choice(ToolChoice::Required),
            ),
        );
    }
    Ok(runners)
}

async fn run_server(
    server: config::ServerConfig,
    models: Vec<config::ModelConfig>,
    providers: Vec<config::ProviderConfig>,
    tools: config::ToolsConfig,
    agent: config::MustAgentConfig,
) -> anyhow::Result<()> {
    let runners = build_runners(models, providers)?;
    let runner = |id: &str| {
        runners
            .get(id)
            .cloned()
            .with_context(|| format!("Unknown agent model ID: {id}"))
    };
    let planner_runner = runner(&agent.planner.model)?;
    let research_runner = runner(&agent.research.model)?;
    let gap_judger_runner = runner(&agent.gap_judger.model)?;
    let explorer_runner = runner(&agent.explorer.model)?;
    let synthesizer_runner = runner(&agent.synthesizer.model)?;

    let mut search_tools = DeepResearchTools::default();
    search_tools.add(
        WebSearchTool::new_for_searxng(&tools.web_search.searxng.endpoint)
            .context("Invalid tools.web_search.searxng.endpoint")?,
    );
    search_tools.add(WebFetchTool::default());
    let explorer = ExplorerTool::new_with_system_prompt(
        explorer_runner,
        search_tools,
        agent.explorer.system_prompt,
    )?;
    let mut research_tools = DeepResearchTools::default();
    research_tools.add(explorer);

    let orchestrator = DeepResearchOrchestrator::new(
        ReActAgent::new(
            planner_runner,
            DeepResearchTools::default(),
            agent.planner.system_prompt,
            HashSet::new(),
        )?,
        ReActAgent::new(
            research_runner,
            research_tools,
            agent.research.system_prompt,
            HashSet::new(),
        )?,
        ReActAgent::new(
            gap_judger_runner,
            DeepResearchTools::default(),
            agent.gap_judger.system_prompt,
            HashSet::new(),
        )?,
        ReActAgent::new(
            synthesizer_runner,
            DeepResearchTools::default(),
            agent.synthesizer.system_prompt,
            HashSet::new(),
        )?,
    );
    let host = if server.host.contains(':') && !server.host.starts_with('[') {
        format!("[{}]", server.host)
    } else {
        server.host
    };
    let addr = format!("{host}:{}", server.port);
    tracing::info!(%addr, "Starting server");
    DeepResearchServer::new(orchestrator).run(&addr).await
}
