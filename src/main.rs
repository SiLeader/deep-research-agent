use anyhow::{Context, bail};
use clap::Parser;
use deep_research_agent_tools::ExplorerTool;
use deep_research_api::DeepResearchServer;
use deep_research_arbiter::AgentConcurrencyArbiter;
use deep_research_arbiter::semaphore::SemaphoreConcurrencyArbiter;
use deep_research_orchestrator::DeepResearchOrchestrator;
use deep_research_react_agent::ReActAgent;
use deep_research_reranker::DeepResearchReranker;
use deep_research_runner::OneshotRunner;
use deep_research_tools::tools::search_fetched::SearchFetchedTool;
use deep_research_tools::tools::{web_fetch::WebFetchTool, web_search::WebSearchTool};
use deep_research_tools::{
    DeepResearchTools,
    fetched::{Embedder, FetchedDb, Reranker},
};
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

#[derive(Clone)]
struct ModelRuntime {
    client: Client,
    provider_type: config::ProviderType,
    request_timeout_secs: u64,
}

struct ModelServices {
    models: HashMap<String, ModelRuntime>,
    arbiter: Arc<dyn AgentConcurrencyArbiter>,
}

impl ModelServices {
    fn runners(&self) -> anyhow::Result<HashMap<String, OneshotRunner>> {
        self.models
            .iter()
            .map(|(id, runtime)| {
                Ok((
                    id.clone(),
                    OneshotRunner::new(
                        id.clone(),
                        runtime.client.clone(),
                        self.arbiter.clone(),
                        ChatOptions::default().with_tool_choice(ToolChoice::Required),
                    )
                    .with_request_timeout(runtime.request_timeout_secs)?,
                ))
            })
            .collect()
    }
}

fn build_model_services(
    models: Vec<config::ModelConfig>,
    providers: Vec<config::ProviderConfig>,
) -> anyhow::Result<ModelServices> {
    let mut provider_map = HashMap::new();
    for provider in providers {
        let id = provider.id.clone();
        if provider_map.insert(id.clone(), provider).is_some() {
            bail!("Duplicate provider ID: {id}");
        }
    }

    let mut limits = HashMap::new();
    for model in &models {
        anyhow::ensure!(
            model.request_timeout_secs > 0,
            "Invalid request_timeout_secs for model: {}",
            model.id
        );
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
    let mut clients = HashMap::new();
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
        clients.insert(
            model.id,
            ModelRuntime {
                client,
                provider_type: provider.provider_type.clone(),
                request_timeout_secs: model.request_timeout_secs,
            },
        );
    }
    Ok(ModelServices {
        models: clients,
        arbiter,
    })
}

#[derive(Clone, Default)]
struct RetrievalModels {
    embedder: Option<Embedder>,
    reranker: Option<Reranker>,
}

impl RetrievalModels {
    fn new(settings: &config::FetchedSettings, services: &ModelServices) -> anyhow::Result<Self> {
        settings.validate()?;
        let embedder = settings
            .embedding
            .as_ref()
            .map(|embedding| {
                let runtime = services.models.get(&embedding.model).with_context(|| {
                    format!(
                        "Unknown tools.fetched.embedding.model ID: {}",
                        embedding.model
                    )
                })?;
                anyhow::ensure!(
                    matches!(runtime.provider_type, config::ProviderType::OpenAI),
                    "tools.fetched.embedding.model requires an OpenAI-compatible provider"
                );
                Embedder::new(embedding.model.clone(), runtime.client.clone())
                    .with_arbiter(services.arbiter.clone())
                    .with_request_timeout(embedding.request_timeout_secs)
            })
            .transpose()?;
        let reranker = settings
            .reranker
            .as_ref()
            .map(|reranker| {
                let client = DeepResearchReranker::new_with_options(
                    reranker.endpoint.clone(),
                    reranker.api_key_env.clone(),
                    reranker.request_timeout_secs,
                    reranker.max_concurrency,
                )
                .context("Invalid tools.fetched.reranker configuration")?;
                Ok::<_, anyhow::Error>(Reranker::new(reranker.model.clone(), client))
            })
            .transpose()?;
        Ok(Self { embedder, reranker })
    }
}

async fn build_search_tools(
    tools: config::ToolsConfig,
    retrieval: RetrievalModels,
) -> anyhow::Result<DeepResearchTools> {
    tools.fetched.validate()?;
    let db = Arc::new(
        FetchedDb::new(
            tools.fetched.search.chunk_size,
            retrieval.embedder,
            retrieval.reranker,
        )
        .await?,
    );
    let mut registry = DeepResearchTools::default();
    registry.add(
        WebSearchTool::new_for_searxng_with_limits(
            &tools.web_search.searxng.endpoint,
            tools.web_search.searxng.limits,
            db.clone(),
        )
        .context("Invalid tools.web_search.searxng.endpoint")?,
    );
    registry.add(WebFetchTool::new(tools.web_fetch, db.clone())?);
    registry.add(SearchFetchedTool::new(db, tools.fetched.search)?);
    Ok(registry)
}

async fn run_server(
    server: config::ServerConfig,
    models: Vec<config::ModelConfig>,
    providers: Vec<config::ProviderConfig>,
    tools: config::ToolsConfig,
    agent: config::MustAgentConfig,
) -> anyhow::Result<()> {
    let services = build_model_services(models, providers)?;
    let retrieval = RetrievalModels::new(&tools.fetched, &services)?;
    let runners = services.runners()?;
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

    // Validate and construct once at startup to fail early on invalid tool settings.
    // This registry is dropped; each Explorer call receives its own fresh database.
    drop(build_search_tools(tools.clone(), retrieval.clone()).await?);
    let explorer = ExplorerTool::new_with_tools_factory(
        explorer_runner,
        move || build_search_tools(tools.clone(), retrieval.clone()),
        agent.explorer.system_prompt,
    )?
    .with_max_llm_calls(agent.max_llm_calls)?;
    let mut research_tools = DeepResearchTools::default();
    research_tools.add(explorer);

    let orchestrator = DeepResearchOrchestrator::new(
        ReActAgent::new(
            planner_runner,
            DeepResearchTools::default(),
            agent.planner.system_prompt,
            HashSet::new(),
        )?
        .with_max_llm_calls(agent.max_llm_calls)?,
        ReActAgent::new(
            research_runner,
            research_tools,
            agent.research.system_prompt,
            HashSet::new(),
        )?
        .with_max_llm_calls(agent.max_llm_calls)?,
        ReActAgent::new(
            gap_judger_runner,
            DeepResearchTools::default(),
            agent.gap_judger.system_prompt,
            HashSet::new(),
        )?
        .with_max_llm_calls(agent.max_llm_calls)?,
        ReActAgent::new(
            synthesizer_runner,
            DeepResearchTools::default(),
            agent.synthesizer.system_prompt,
            HashSet::new(),
        )?
        .with_max_llm_calls(agent.max_llm_calls)?,
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

#[cfg(test)]
mod retrieval_tests;
