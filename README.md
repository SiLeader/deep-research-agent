# Deep Research Agent

[日本語](README_ja.md)

A Rust service for LLM-driven web research. It creates a research plan, investigates its steps with a ReAct agent and
web tools, checks the findings for gaps, and runs a final synthesis. The HTTP API returns plans as JSON and streams
research progress using Server-Sent Events (SSE).

## How it works

1. **Planner** turns a question into research goals and a report goal, or revises an existing plan.
2. **Researcher** investigates each goal through an **explorer**, which uses SearXNG search and HTTP page fetching.
3. **Gap judger** reviews the findings and references. A rejected step is retried, with a limit of 10 research/review
   cycles per step.
4. **Synthesizer** combines the accepted findings according to the report goal.

Research steps run concurrently. Model requests share a concurrency limit per configured model ID. Each role can use its
own model and system prompt.

## Requirements

- Rust and Cargo with Rust 2024 edition support and compatibility with the locked dependencies.
- An LLM endpoint supporting tool calls, configured with provider type `OpenAI` or `Anthropic`.
- A reachable SearXNG search endpoint that permits JSON responses.
- `curl` for the API examples and `jq` to wrap a saved plan into a research request.

## Quick start

From the repository root, create a local configuration if you do not already have one:

```sh
cp config.example.toml config.local.toml
```

Edit `config.local.toml`:

- Replace `models.name = "your-model-name"` with the model name accepted by your provider.
- Set `providers.api_key_env` to the environment variable containing your API key.
- Set `providers.endpoint` if you need a custom API base URL, including its required path and trailing slash.
- Set `tools.web_search.searxng.endpoint` to the full search URL, such as `http://127.0.0.1:8888/search`.

For the example provider, export your key and start the server:

```sh
export OPENAI_API_KEY='your-api-key'
RUST_LOG=info cargo run --release -- --config config.local.toml
```

The example configuration binds to `127.0.0.1:8080`. Without `--config`, the binary reads
`/etc/deep-research-agent/config.toml`. Logs are JSON; `RUST_LOG` controls the log filter. `config.local.toml` is
ignored by Git.

Check the server:

```sh
curl --fail-with-body http://127.0.0.1:8080/version
```

## Configuration

See [config.example.toml](config.example.toml) for the full starting configuration.

| Setting                                                                                 | Meaning                                                                                              |
|-----------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------|
| `server.host`, `server.port`                                                            | Listen address; defaults to `127.0.0.1:8080`.                                                        |
| `agent.model`                                                                           | Default model ID for all roles.                                                                      |
| `agent.max_llm_calls`                                                                   | Maximum LLM calls per agent invocation, including Explorer; defaults to `30` and must be positive.   |
| `agent.<role>.model`                                                                    | Model ID override for `planner`, `research`, `gap_judger`, `explorer`, or `synthesizer`.             |
| `agent.<role>.system_prompt`                                                            | Inline system prompt override; omitted prompts use embedded files in `src/assets/`.                  |
| `models[].id`                                                                           | Unique model ID referenced by agent roles.                                                           |
| `models[].provider`                                                                     | ID of a configured provider.                                                                         |
| `models[].name`                                                                         | Model name sent to the provider.                                                                     |
| `models[].max_concurrency`                                                              | Maximum simultaneous requests for this model ID; defaults to `1` and must be positive.               |
| `models[].request_timeout_secs`                                                         | Deadline per chat request after acquiring a model slot; defaults to `120` seconds and must be positive. Embeddings use their own retrieval timeout. |
| `providers[].id`                                                                        | Unique provider ID.                                                                                  |
| `providers[].type`                                                                      | `OpenAI` or `Anthropic` (case-sensitive).                                                            |
| `providers[].api_key_env`                                                               | Environment variable used for authentication.                                                        |
| `providers[].endpoint`                                                                  | Optional API base URL override.                                                                      |
| `tools.web_search.searxng.endpoint`                                                     | Full SearXNG search URL; the client adds `format=json` and `q`.                                      |
| `tools.web_fetch.connect_timeout_secs`, `tools.web_search.searxng.connect_timeout_secs` | Connection timeout; defaults to `10` seconds.                                                        |
| `tools.web_fetch.request_timeout_secs`, `tools.web_search.searxng.request_timeout_secs` | Total HTTP request timeout; defaults to `60` seconds. Fetch includes all redirects in this deadline. |
| `tools.web_fetch.max_body_bytes`, `tools.web_search.searxng.max_body_bytes`             | Maximum response body size; defaults to `2097152` bytes (2 MiB).                                     |

Model selection uses the role override first, then `agent.model`. If neither is set, `gap_judger` and `explorer` fall
back to the resolved research model. The planner, research, and synthesizer roles require a role model or `agent.model`.

Each Explorer invocation owns a fresh in-memory `FetchedDb`, released when it ends.
`search_sources` saves available snippets; `fetch` saves successful HTML (converted to Markdown), plain text, and Markdown.
Call `search_fetched` with `query` and `top_k` (null for the default) to retrieve evidence chunks with `url`, `content`, and `score`.
The server defaults to lexical search. A supplementary character index supports keywords and phrases inside unsegmented Japanese and Chinese text, while returned evidence retains the original content. Optional embedding enables hybrid lexical/vector retrieval; optional reranking reorders the candidates in either mode.
`[tools.fetched]` controls `chunk_size` (characters, default 1024), `default_top_k` (5), and `max_top_k` (20).
Chunk size must be positive and `0 < default_top_k <= max_top_k`. Requested counts are capped at the maximum.
Wait for fetch results before searching newly saved content: calls in the same tool batch may execute concurrently.

Migration: replace reads of `fetch.content` with `search_fetched`. Fetch now returns
`status_code`, `url` (after redirects), `content_type`, and `stored`. HTTP errors, unsupported or missing Content-Type,
and empty content are not indexed. Storage failures become tool errors. Update custom Explorer prompts accordingly.
Rust callers must pass a shared `Arc<FetchedDb>` to `WebSearchTool` and `WebFetchTool` constructors.
Replace `WebFetchTool::default()` with `new(limits, db)`. Use `ExplorerTool::new_with_tools_factory`
for per-invocation tools; existing Explorer constructors share the supplied registry.

To enable embeddings, register an OpenAI-compatible embedding model in `[[models]]` and reference its ID
in `[tools.fetched.embedding].model`. The existing `[[providers]]` entry supplies the endpoint and API key.
Prefer a separate model ID for embeddings. A model ID's `max_concurrency` is shared by chat, embeddings, and all Explorer invocations.
The reranker `model` is the service model name, not a `[[models]]` ID.
Its `endpoint` is a Cohere-compatible API base URL; trailing slashes are normalized before appending `/rerank`.
Omit `api_key_env` for unauthenticated services, or name the environment variable containing the Bearer token.
Both `request_timeout_secs` values default to 60 and must be positive. The reranker `max_concurrency` defaults to 1 and is shared across explorations.
Invalid retrieval settings fail at startup. Missing credentials, API errors, and timeouts become tool errors; there is no automatic fallback to lexical search.

```toml
[[models]]
id = "embedding"
provider = "openai"
name = "your-embedding-model"
max_concurrency = 1

[tools.fetched.embedding]
model = "embedding"
request_timeout_secs = 60

[tools.fetched.reranker]
endpoint = "http://127.0.0.1:8001/v1/"
model = "your-reranker-model"
# api_key_env = "RERANKER_API_KEY"
request_timeout_secs = 60
max_concurrency = 1
```

## HTTP API

| Method | Path                         | Response                                    |
|--------|------------------------------|---------------------------------------------|
| `GET`  | `/version`                   | Package version and supported API versions. |
| `POST` | `/api/v1/deep/research/plan` | Research plan as JSON.                      |
| `POST` | `/api/v1/deep/research`      | Research events as an SSE stream.           |

### Create a plan

```sh
curl --fail-with-body http://127.0.0.1:8080/api/v1/deep/research/plan \
  -H 'Content-Type: application/json' \
  -d '{"prompt":"Compare approaches to long-duration energy storage."}' \
  -o plan.json
```

The response has this structure (goals below are illustrative):

```json
{
  "research_plans": [
    {
      "goal": "Compare major storage technologies.",
      "scope": "Stationary storage; exclude vehicle applications.",
      "questions": [
        "What are the advantages and limitations of major technologies?"
      ]
    }
  ],
  "report_plan": {
    "goal": "Write an evidence-based comparison.",
    "sections": [
      {
        "heading": "Technology comparison",
        "focus": "Compare advantages and limitations."
      }
    ]
  }
}
```

To revise a plan, send `prompt` and an optional `previous_plan` containing the complete plan object to the same
endpoint. The response is the complete revised plan. Planning failures return HTTP `500`.

### Run research

Wrap the saved plan in a `plan` field and submit it:

```sh
jq '{plan: .}' plan.json > research-request.json
curl --fail-with-body -N http://127.0.0.1:8080/api/v1/deep/research \
  -H 'Content-Type: application/json' \
  --data-binary @research-request.json
```

Each SSE data frame contains JSON with `model`, `phase`, and `data`. `model` is the configured model ID. Phases are
`Researching`, `GapJudging`, `ResearchStepCompleted`, `Synthesizing`, `Synthesized`, and `Failed`. Events from
concurrent research steps may interleave.

Example completed research step:

```text
data: {"model":"default","phase":"ResearchStepCompleted","data":{"findings":[{"question":"What are the advantages and limitations of major technologies?","answer":"Findings...","status":"supported","references":[{"source":"https://example.com/source","content":"Supporting evidence..."}]}],"limitations":[]}}

```

The server sends `: keep-alive` comments every 15 seconds while waiting for events. Successful completion emits
`Synthesized`; a research or synthesis failure emits `Failed` with `data.error` and closes the stream. Clients must
inspect the events for failures even when the HTTP status is `200`. Disconnecting cancels the associated research task.

`Synthesized.data` contains the final report with `title`, `summary`, `sections` (`heading`, `content`, `sources`), and
`limitations`. See [stage contracts and migration](docs/stage-contracts.md) for the full payloads. The previous plan and
research-result formats are incompatible.

## Current limitations

- HTML is converted to Markdown before indexing; JavaScript is not executed. PDF and other binary formats are not indexed.
- Fetch permits only public HTTP (S) destinations, validates DNS answers and redirects, and bypasses environment
  proxies. Internal, loopback, link-local, and reserved IP ranges are rejected. Configured LLM and SearXNG endpoints may
  still be internal.
- SearXNG results require `url`, `title`, and `score`; publication dates (`publishedDate`) may be missing or null.
  Returned pages expose `published_date` as a UTC timestamp or null.
- Each agent invocation is bounded by `agent.max_llm_calls`. The separate limit of 10 research/review cycles remains.
  Chat requests time out according to `models[].request_timeout_secs` and release their model concurrency slot; the deadline does not include waiting for that slot.
  Web timeouts and body limits are configurable; exceeding them returns a tool error for the agent to handle.

## Development

```sh
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
```

| Crate                        | Responsibility                                              |
|------------------------------|-------------------------------------------------------------|
| `deep-research-agent`        | Executable, configuration, and agent wiring.                |
| `deep-research-api`          | Actix Web endpoints and SSE responses.                      |
| `deep-research-orchestrator` | Planning, research coordination, gap review, and synthesis. |
| `deep-research-react-agent`  | ReAct execution and agent events.                           |
| `deep-research-agent-tools`  | Explorer agent exposed as a tool.                           |
| `deep-research-tools`        | Tool interfaces, search, fetch, and marker tools.           |
| `deep-research-runner`       | LLM request execution.                                      |
| `deep-research-arbiter`      | Model concurrency control.                                  |

## License

[Apache License 2.0](LICENSE).
