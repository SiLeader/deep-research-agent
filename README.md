# Deep Research Agent

[日本語](README_ja.md)

A Rust service for LLM-driven web research. It creates a research plan, investigates its steps with a ReAct agent and web tools, checks the findings for gaps, and runs a final synthesis. The HTTP API returns plans as JSON and streams research progress using Server-Sent Events (SSE).

## How it works

1. **Planner** turns a question into research goals and a report goal, or revises an existing plan.
2. **Researcher** investigates each goal through an **explorer**, which uses SearXNG search and HTTP page fetching.
3. **Gap judger** reviews the findings and references. A rejected step is retried, with a limit of 10 research/review cycles per step.
4. **Synthesizer** combines the accepted findings according to the report goal.

Research steps run concurrently. Model requests share a concurrency limit per configured model ID. Each role can use its own model and system prompt.

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

The example configuration binds to `127.0.0.1:8080`. Without `--config`, the binary reads `/etc/deep-research-agent/config.toml`. Logs are JSON; `RUST_LOG` controls the log filter. `config.local.toml` is ignored by Git.

Check the server:

```sh
curl --fail-with-body http://127.0.0.1:8080/version
```

## Configuration

See [config.example.toml](config.example.toml) for the full starting configuration.

| Setting | Meaning |
| --- | --- |
| `server.host`, `server.port` | Listen address; defaults to `127.0.0.1:8080`. |
| `agent.model` | Default model ID for all roles. |
| `agent.<role>.model` | Model ID override for `planner`, `research`, `gap_judger`, `explorer`, or `synthesizer`. |
| `agent.<role>.system_prompt` | Inline system prompt override; omitted prompts use embedded files in `src/assets/`. |
| `models[].id` | Unique model ID referenced by agent roles. |
| `models[].provider` | ID of a configured provider. |
| `models[].name` | Model name sent to the provider. |
| `models[].max_concurrency` | Maximum simultaneous requests for this model ID; defaults to `1` and must be positive. |
| `providers[].id` | Unique provider ID. |
| `providers[].type` | `OpenAI` or `Anthropic` (case-sensitive). |
| `providers[].api_key_env` | Environment variable used for authentication. |
| `providers[].endpoint` | Optional API base URL override. |
| `tools.web_search.searxng.endpoint` | Full SearXNG search URL; the client adds `format=json` and `q`. |

Model selection uses the role override first, then `agent.model`. If neither is set, `gap_judger` and `explorer` fall back to the resolved research model. The planner, research, and synthesizer roles require a role model or `agent.model`.

## HTTP API

| Method | Path | Response |
| --- | --- | --- |
| `GET` | `/version` | Package version and supported API versions. |
| `POST` | `/v1/deep/research/plan` | Research plan as JSON. |
| `POST` | `/v1/deep/research` | Research events as an SSE stream. |

### Create a plan

```sh
curl --fail-with-body http://127.0.0.1:8080/v1/deep/research/plan \
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

To revise a plan, send `prompt` and an optional `previous_plan` containing the complete plan object to the same endpoint. The response is the complete revised plan. Planning failures return HTTP `500`.

### Run research

Wrap the saved plan in a `plan` field and submit it:

```sh
jq '{plan: .}' plan.json > research-request.json
curl --fail-with-body -N http://127.0.0.1:8080/v1/deep/research \
  -H 'Content-Type: application/json' \
  --data-binary @research-request.json
```

Each SSE data frame contains JSON with `model`, `phase`, and `data`. `model` is the configured model ID. Phases are `Researching`, `GapJudging`, `ResearchStepCompleted`, `Synthesizing`, `Synthesized`, and `Failed`. Events from concurrent research steps may interleave.

Example completed research step:

```text
data: {"model":"default","phase":"ResearchStepCompleted","data":{"findings":[{"question":"What are the advantages and limitations of major technologies?","answer":"Findings...","status":"supported","references":[{"source":"https://example.com/source","content":"Supporting evidence..."}]}],"limitations":[]}}

```

The server sends `: keep-alive` comments every 15 seconds while waiting for events. Successful completion emits `Synthesized`; a research or synthesis failure emits `Failed` with `data.error` and closes the stream. Clients must inspect the events for failures even when the HTTP status is `200`. Disconnecting cancels the associated research task.

`Synthesized.data` contains the final report with `title`, `summary`, `sections` (`heading`, `content`, `sources`), and `limitations`. See [stage contracts and migration](docs/stage-contracts.md) for the full payloads. The previous plan and research-result formats are incompatible.

## Current limitations

- Page fetching returns raw response text, including HTML. It does not extract article text or execute JavaScript.
- The SearXNG response parser currently requires every result to include `url`, `title`, `score`, and `published_date`, with a timestamp parseable as `DateTime<Utc>`. Results missing these fields cause deserialization to fail.

## Development

```sh
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
```

| Crate | Responsibility |
| --- | --- |
| `deep-research-agent` | Executable, configuration, and agent wiring. |
| `deep-research-api` | Actix Web endpoints and SSE responses. |
| `deep-research-orchestrator` | Planning, research coordination, gap review, and synthesis. |
| `deep-research-react-agent` | ReAct execution and agent events. |
| `deep-research-agent-tools` | Explorer agent exposed as a tool. |
| `deep-research-tools` | Tool interfaces, search, fetch, and marker tools. |
| `deep-research-runner` | LLM request execution. |
| `deep-research-arbiter` | Model concurrency control. |

## License

[Apache License 2.0](LICENSE).
