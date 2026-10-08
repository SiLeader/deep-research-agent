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
| `server.host`, `server.port`                                                            | Listen address; defaults to `127.0.0.1:8080`. In a container, set `host = "0.0.0.0"` to accept outside connections. |
| `server.api_key_env`                                                                    | Optional environment variable holding a token; `/api/` routes then require `Authorization: Bearer <token>`. |
| `agent.model`                                                                           | Default model ID for all roles.                                                                      |
| `agent.max_llm_calls`                                                                   | Maximum LLM calls per agent invocation, including Explorer; defaults to `30` and must be positive.   |
| `agent.max_research_loops`                                                              | Research/review cycles per research step before it fails; defaults to `10`.                         |
| `agent.max_total_llm_calls`                                                             | LLM requests per research or planning request across all agents, Explorer, and retries; defaults to `2000`. |
| `agent.research_timeout_secs`                                                           | Deadline for a whole research request, including synthesis, and for each planning request; defaults to `3600`. |
| `agent.max_concurrent_requests`                                                         | Planning and research requests running at once; further requests receive HTTP `503`. Defaults to `4`. |
| `agent.max_tool_context_chars`                                                          | Tool output kept in each agent's conversation; older outputs are elided first. Defaults to `200000`. |
| `agent.<role>.model`                                                                    | Model ID override for `planner`, `research`, `gap_judger`, `explorer`, or `synthesizer`.             |
| `agent.<role>.system_prompt`                                                            | Inline system prompt override; omitted prompts use embedded files in `src/assets/`.                  |
| `models[].id`                                                                           | Unique model ID referenced by agent roles.                                                           |
| `models[].provider`                                                                     | ID of a configured provider.                                                                         |
| `models[].name`                                                                         | Model name sent to the provider.                                                                     |
| `models[].max_concurrency`                                                              | Maximum simultaneous requests for this model ID; defaults to `1` and must be positive.               |
| `models[].request_timeout_secs`                                                         | Deadline per chat request after acquiring a model slot; defaults to `120` seconds and must be positive. Embeddings use their own retrieval timeout. |
| `models[].max_retries`                                                                  | Retries for timeouts, connection errors, and HTTP 408/409/425/429/5xx with exponential backoff (honoring `Retry-After`); defaults to `3`. |
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
Both `request_timeout_secs` values default to 60 and must be positive. The embedding `batch_size` (inputs per request) defaults to 64. The reranker `max_concurrency` defaults to 1 and is shared across explorations.
If saving search snippets fails, `search_sources` still returns its pages with `snippets_saved: false`.
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
endpoint. The response is the complete revised plan. A blank `prompt`, a `prompt` over 20,000 characters, or an
invalid `previous_plan` returns HTTP `400`; planning failures return HTTP `500` and are logged.

Plans may contain at most 20 research steps, 20 questions per step, and 30 report sections, with at most 4,000
characters per string. Larger plans are rejected with HTTP `400`. When `agent.max_concurrent_requests` requests are
already running, the plan and research endpoints return HTTP `503` with `Retry-After`.

When `server.api_key_env` is set, add `-H "Authorization: Bearer $API_KEY"` to `/api/` requests. Unauthenticated
requests receive HTTP `401`. Without it the server logs a warning at startup; do not expose such a server publicly.

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
concurrent research steps may interleave; `Researching`, `GapJudging`, and `ResearchStepCompleted` carry `step`, the
0-based index in `research_plans`. `Failed` carries `step` when a specific step failed.

Example completed research step (`findings[i]` answers `questions[i]` of that step):

```text
data: {"model":"default","step":0,"phase":"ResearchStepCompleted","data":{"findings":[{"answer":"Findings...","status":"supported","references":[{"source":"https://example.com/source","content":"Supporting evidence..."}]}],"limitations":[]}}

```

If an agent submits output that fails validation (wrong number of findings, an unknown cited source, and so on), the
error is returned to the model as the tool response and it may resubmit within `agent.max_llm_calls`.
Explorer, research, gap review, and final-report outputs are each limited to 200,000 serialized JSON characters
(including keys and escapes). Oversized submissions must be shortened and resubmitted.
Text-only replies are retried within the same call limit when a submission tool is required.
Call `submit` in a separate turn after reviewing other tool results; mixed batches execute the other tools and reject the submission.
API JSON bodies are limited to 16 MiB to accommodate the largest valid escaped plans; larger bodies receive HTTP `413`.
Chat and embedding provider error details are logged on the server, not returned in SSE or tool responses.

The server sends `: keep-alive` comments every 15 seconds while waiting for events. Successful completion emits
`Synthesized`; a research or synthesis failure emits `Failed` with `data.error` and closes the stream. Clients must
inspect the events for failures even when the HTTP status is `200`. Disconnecting cancels the associated research task.

`Synthesized.data` contains the final report with `title`, `summary`, `sections` (`heading`, `content`, `sources`), and
`limitations`. See [stage contracts and migration](docs/stage-contracts.md) for the full payloads. The previous plan and
research-result formats are incompatible.

## Current limitations

- HTML is converted to Markdown before indexing; JavaScript is not executed. PDF and other binary formats are not indexed.
  Text is decoded using the byte-order mark, the Content-Type charset, or an HTML `<meta>` charset, in that order.
- Fetch permits only public HTTP (S) destinations, validates DNS answers and redirects, and bypasses environment
  proxies. Internal, loopback, link-local, and reserved IP ranges are rejected. Configured LLM and SearXNG endpoints may
  still be internal.
- SearXNG results require `url`, `title`, and `score`; publication dates (`publishedDate`) may be missing or null.
  Dates without a UTC offset are treated as UTC, and unparsable dates become null.
  Returned pages expose `published_date` as a UTC timestamp or null.
- Each agent invocation is bounded by `agent.max_llm_calls`; research steps by `agent.max_research_loops`; each
  research request by `agent.max_total_llm_calls` and `agent.research_timeout_secs`. Chat requests time out according to `models[].request_timeout_secs` and release their model concurrency slot; the deadline does not include waiting for that slot.
  Web timeouts and body limits are configurable; exceeding them returns a tool error for the agent to handle.
  The fetch request timeout covers retrieval and redirects only; indexing a page, including embedding, is bounded by
  `tools.fetched.embedding.request_timeout_secs` per batch.

## Development

```sh
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
```

### CI and releases

The **CI** workflow runs formatting, Clippy, and workspace tests on pushes and pull requests. It can also be run manually.
Workflows use only GitHub-official Actions and Docker's verified-publisher Actions.
Rust, Cargo, and `gh` use the runner's preinstalled tools.
The container build uses the Rust image specified in `Dockerfile`.

Run **Release** from the Actions tab on a branch, supplying `version` as SemVer without a `v` prefix
(for example, `0.2.0` or `0.2.0-rc.1`). Build metadata (`+...`) is not supported because it is invalid in container tags.
If the value differs from `Cargo.toml`'s `workspace.package.version`, the workflow opens a PR against the selected
branch updating `Cargo.toml` and `Cargo.lock`, then stops without publishing. Merge the PR and run Release again
with the same version.

When versions match, CI must pass before the workflow pushes `ghcr.io/sileader/deep-research-agent` with
`latest`, the exact version, and the full commit SHA as tags, then creates a GitHub Release tagged `v<version>`
at that commit. Versions containing a prerelease suffix create prereleases. Existing release tags are rejected.

The workflow uses `GITHUB_TOKEN`; enable **Allow GitHub Actions to create and approve pull requests** in
Settings > Actions > General. The token also needs write access to the GHCR package if it already exists.
PRs created with this token do not automatically trigger CI; run CI manually on the version-update branch
if checks are required before merging.

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
