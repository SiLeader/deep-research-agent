# Deep Research Agent

[English](README.md)

LLM を使った Web 調査を行う Rust 製サービスです。調査計画を作成し、ReAct エージェントと Web
ツールで各項目を調査した後、情報の不足を確認し、最後に結果を統合します。HTTP API は計画を JSON で返し、調査の進捗を
Server-Sent Events（SSE）で配信します。

## 動作の流れ

1. **Planner** が質問から調査項目とレポートの目標を作成します。既存の計画の修正にも対応します。
2. **Researcher** が各項目を調査します。 **Explorer** を通じて SearXNG による検索と HTTP によるページ取得を行います。
3. **Gap judger** が調査結果と参照情報を確認します。不十分と判断した項目は再調査し、調査と確認のサイクルを項目ごとに最大
   10 回実行します。
4. **Synthesizer** が承認された結果をレポートの目標に沿って統合します。

調査項目は並行して実行されます。モデルへのリクエストは、設定したモデル ID ごとに同時実行数を制限します。各ロールには個別のモデルとシステムプロンプトを指定できます。

## 必要なもの

- Rust 2024 エディションと、ロックされた依存関係に対応する Rust・Cargo。
- ツール呼び出しに対応した LLM エンドポイント。プロバイダー種別には `OpenAI` または `Anthropic` を指定します。
- JSON レスポンスを許可した、接続可能な SearXNG 検索エンドポイント。
- API の使用例用の `curl` と、保存した計画を調査リクエストに変換するための `jq`。

## クイックスタート

ローカル設定がまだない場合は、リポジトリのルートで作成します。

```sh
cp config.example.toml config.local.toml
```

`config.local.toml` を編集します。

- `models.name = "your-model-name"` を、プロバイダーで利用できるモデル名に変更します。
- `providers.api_key_env` に、API キーを格納する環境変数名を指定します。
- 独自の API ベース URL を使う場合は、必要なパスと末尾のスラッシュを含めて `providers.endpoint` を設定します。
- `tools.web_search.searxng.endpoint` に、`http://127.0.0.1:8888/search` などの検索 URL 全体を指定します。

サンプル設定のプロバイダーを使う場合は、キーを環境変数に設定して起動します。

```sh
export OPENAI_API_KEY='your-api-key'
RUST_LOG=info cargo run --release -- --config config.local.toml
```

サンプル設定では `127.0.0.1:8080` で待ち受けます。`--config` を省略すると、`/etc/deep-research-agent/config.toml`
を読み込みます。ログは JSON 形式で、`RUST_LOG` で出力フィルターを指定できます。`config.local.toml` は Git の管理対象から除外されています。

起動を確認します。

```sh
curl --fail-with-body http://127.0.0.1:8080/version
```

## 設定

設定の雛形は [config.example.toml](config.example.toml) を参照してください。

| 設定                                | 内容                                                                                  |
|-------------------------------------|---------------------------------------------------------------------------------------|
| `server.host`, `server.port`        | 待ち受け先。既定値は `127.0.0.1:8080`。                                               |
| `agent.max_llm_calls`               | Explorerを含む各エージェント実行のLLM呼び出し上限。既定値は `30`。正の値が必要。      |
| `agent.model`                       | 全ロールの既定モデル ID。                                                             |
| `agent.<role>.model`                | `planner`、`research`、`gap_judger`、`explorer`、`synthesizer` のモデル ID を上書き。 |
| `agent.<role>.system_prompt`        | システムプロンプトを文字列で指定。省略時は `src/assets/` の組み込みファイルを使用。   |
| `models[].id`                       | ロールから参照する、一意のモデル ID。                                                 |
| `models[].provider`                 | 設定済みプロバイダーの ID。                                                           |
| `models[].name`                     | プロバイダーに送信するモデル名。                                                      |
| `models[].max_concurrency`          | このモデル ID の最大同時リクエスト数。既定値は `1`。正の値が必要。                    |
| `models[].request_timeout_secs`     | 同時実行枠の取得後、各チャットリクエストに適用する期限。既定値は `120` 秒。正の値が必要。埋め込みには検索設定の期限を使用。 |
| `providers[].id`                    | 一意のプロバイダー ID。                                                               |
| `providers[].type`                  | `OpenAI` または `Anthropic`。大文字・小文字を区別。                                   |
| `providers[].api_key_env`           | 認証に使う環境変数名。                                                                |
| `providers[].endpoint`              | API ベース URL の上書き。省略可能。                                                   |
| `tools.web_search.searxng.endpoint` | SearXNG の検索 URL 全体。クライアントが `format=json` と `q` を追加。                 |

モデルの選択では、ロールごとの指定、`agent.model` の順で優先します。両方が未設定の場合、`gap_judger` と `explorer` は
research ロールで解決されたモデルを使います。planner、research、synthesizer には、ロールごとのモデルか `agent.model`
の指定が必要です。

Web取得の上限は `[tools.web_fetch]`、検索の上限は `[tools.web_search.searxng]` で設定します。`connect_timeout_secs`
は既定で10秒、`request_timeout_secs` は60秒、`max_body_bytes` は2097152バイト（2
MiB）です。すべて正の値が必要です。fetchの60秒にはリダイレクト先の取得も含みます。

取得データはExplorer呼び出しごとのインメモリ `FetchedDb` に保存され、終了時に解放されます。
`search_sources` は利用可能なスニペットを保存し、`fetch` は成功したHTML（Markdownに変換）、プレーンテキスト、Markdownを保存します。
`search_fetched` に `query` と `top_k`（nullで既定値）を渡すと、関連チャンクの `url`・`content`・`score` を取得できます。
既定は全文検索です。補助の文字索引により、空白で区切られていない日本語・中国語の文中にあるキーワードや語句も検索できます。返される根拠は元の本文です。埋め込みを指定すると全文・ベクトルのハイブリッド検索になり、再ランキングはどちらの検索方式にも追加できます。
`[tools.fetched]` の `chunk_size` は文字数で既定1024、`default_top_k` は5、`max_top_k` は20です。
`0 < default_top_k <= max_top_k` と正のチャンクサイズが必要です。指定件数は最大値で制限します。
取得と検索を同じ並行ツール呼び出しに入れると保存前に検索する場合があるため、取得結果を待ってから検索してください。

移行時は、`fetch` の `content` を読む処理を `search_fetched` に置き換えてください。新しい取得結果は
`status_code`・`url`（リダイレクト後）・`content_type`・`stored` を返します。HTTPエラー、未対応・未指定のContent-Type、
空本文は保存されません。保存失敗はツールエラーになります。カスタムExplorerプロンプトもこの手順に更新してください。
Rust APIでは `WebSearchTool` と `WebFetchTool` のコンストラクタに共有する `Arc<FetchedDb>` を渡します。
`WebFetchTool::default()` は廃止し、`new(limits, db)` を使います。実行ごとの生成には
`ExplorerTool::new_with_tools_factory` を使用します。従来のExplorerコンストラクタは渡したツールを共有します。

埋め込みを有効にする場合は、OpenAI互換の埋め込みモデルを `[[models]]` に追加し、
`[tools.fetched.embedding]` の `model` にそのIDを指定します。接続先とAPIキーは既存の `[[providers]]` を使います。
チャット用モデルとは別のIDを推奨します。同じIDの `max_concurrency` はチャットと埋め込み、全Explorer実行で共有します。
再ランキングの `model` はサービスに渡すモデル名で、`[[models]]` のIDではありません。
`endpoint` はCohere互換APIのベースURLです。末尾のスラッシュを正規化して `/rerank` を追加します。
`api_key_env` を省略すると認証ヘッダーを送りません。指定時は環境変数からBearerトークンを読みます。
両方の `request_timeout_secs` は既定60秒で正の値が必要です。再ランキングの `max_concurrency` は既定1で、全Explorer実行で共有します。
設定ミスは起動時にエラーになります。APIキーの欠落、APIエラー、タイムアウトはツールエラーになり、全文検索への自動切替は行いません。

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

| メソッド | パス                         | レスポンス                                    |
|----------|------------------------------|-----------------------------------------------|
| `GET`    | `/version`                   | パッケージのバージョンと対応 API バージョン。 |
| `POST`   | `/api/v1/deep/research/plan` | JSON 形式の調査計画。                         |
| `POST`   | `/api/v1/deep/research`      | SSE 形式の調査イベント。                      |

### 計画の作成

```sh
curl --fail-with-body http://127.0.0.1:8080/api/v1/deep/research/plan \
  -H 'Content-Type: application/json' \
  -d '{"prompt":"長時間のエネルギー貯蔵に使える方式を比較してください。"}' \
  -o plan.json
```

レスポンスは次の構造です。以下の目標は説明用の例です。

```json
{
  "research_plans": [
    {
      "goal": "主要な蓄電技術の特徴を比較する",
      "scope": "定置型蓄電。車載用途は除く",
      "questions": [
        "主要な技術の利点と制約は？"
      ]
    }
  ],
  "report_plan": {
    "goal": "根拠のある比較レポート",
    "sections": [
      {
        "heading": "技術の比較",
        "focus": "利点と制約を比較する"
      }
    ]
  }
}
```

計画を修正する場合は、同じエンドポイントに `prompt` と、省略可能な `previous_plan` を送信します。`previous_plan`
には計画オブジェクト全体を指定します。レスポンスには修正後の計画全体が返ります。計画作成に失敗した場合は HTTP `500` を返します。

### 調査の実行

保存した計画を `plan` フィールドで包み、送信します。

```sh
jq '{plan: .}' plan.json > research-request.json
curl --fail-with-body -N http://127.0.0.1:8080/api/v1/deep/research \
  -H 'Content-Type: application/json' \
  --data-binary @research-request.json
```

SSE の各データフレームには、`model`、`phase`、`data` を持つ JSON が入ります。`model` は設定したモデル ID です。フェーズは
`Researching`、`GapJudging`、`ResearchStepCompleted`、`Synthesizing`、`Synthesized`、`Failed`
です。並行して実行する調査項目のイベントは混在する場合があります。

調査項目の完了イベントの例です。

```text
data: {"model":"default","phase":"ResearchStepCompleted","data":{"findings":[{"question":"主要な技術の利点と制約は？","answer":"調査結果...","status":"supported","references":[{"source":"https://example.com/source","content":"根拠となる情報..."}]}],"limitations":[]}}

```

イベントを待つ間は、15 秒ごとに `: keep-alive` コメントを送信します。正常終了時は `Synthesized` を出力します。調査や統合に失敗した場合は、
`data.error` を持つ `Failed` を出力してストリームを閉じます。HTTP ステータスが `200`
でも、クライアントはイベント内の失敗を確認する必要があります。接続を切断すると、その調査タスクはキャンセルされます。

`Synthesized.data` には `title`、`summary`、`sections`（`heading`、`content`、`sources`）、`limitations`
を持つ最終レポートが入ります。各ステップの契約と旧形式からの移行方法は [ステップ間のデータ契約](docs/stage-contracts.md)
を参照してください。旧形式の計画と調査結果は互換性がありません。

## 現在の制約

- HTMLはMarkdownへ変換して保存します。JavaScriptの実行やPDFなどのバイナリ形式の索引作成には対応していません。
- fetchは公開HTTP (S)
  URLだけを許可し、DNS解決後のIPとリダイレクト先も検証します。内部・ループバック・リンクローカル・予約済みIPは拒否し、環境変数のプロキシは使いません。設定したLLMとSearXNGの接続先には内部URLを使用できます。
- SearXNG結果の `url`、`title`、`score` は必須ですが、公開日時の `publishedDate` は省略・nullを許容します。ツール出力の
  `published_date` はUTC日時またはnullになります。
- 各エージェント実行は `agent.max_llm_calls` で制限します。調査・レビューの10回制限は別に維持します。Web取得の時間・本文サイズ超過はツールエラーとしてエージェントに返します。
  チャットリクエストは `models[].request_timeout_secs` でタイムアウトし、同時実行枠を解放します。この期限に枠の取得待ちは含みません。

## 開発

```sh
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
```

| クレート                     | 役割                                                         |
|------------------------------|--------------------------------------------------------------|
| `deep-research-agent`        | 実行ファイル、設定、エージェントの組み立て。                 |
| `deep-research-api`          | Actix Web のエンドポイントと SSE レスポンス。                |
| `deep-research-orchestrator` | 計画、調査の制御、不足の確認、結果の統合。                   |
| `deep-research-react-agent`  | ReAct の実行とエージェントイベント。                         |
| `deep-research-agent-tools`  | ツールとして呼び出せる Explorer エージェント。               |
| `deep-research-tools`        | ツールのインターフェース、検索、ページ取得、マーカーツール。 |
| `deep-research-runner`       | LLM リクエストの実行。                                       |
| `deep-research-arbiter`      | モデルの同時実行数の制御。                                   |

## ライセンス

[Apache License 2.0](LICENSE)。
