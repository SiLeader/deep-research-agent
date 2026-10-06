# Deep Research Agent

[English](README.md)

LLM を使った Web 調査を行う Rust 製サービスです。調査計画を作成し、ReAct エージェントと Web ツールで各項目を調査した後、情報の不足を確認し、最後に結果を統合します。HTTP API は計画を JSON で返し、調査の進捗を Server-Sent Events（SSE）で配信します。

## 動作の流れ

1. **Planner** が質問から調査項目とレポートの目標を作成します。既存の計画の修正にも対応します。
2. **Researcher** が各項目を調査します。**Explorer** を通じて SearXNG による検索と HTTP によるページ取得を行います。
3. **Gap judger** が調査結果と参照情報を確認します。不十分と判断した項目は再調査し、調査と確認のサイクルを項目ごとに最大 10 回実行します。
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

サンプル設定では `127.0.0.1:8080` で待ち受けます。`--config` を省略すると、`/etc/deep-research-agent/config.toml` を読み込みます。ログは JSON 形式で、`RUST_LOG` で出力フィルターを指定できます。`config.local.toml` は Git の管理対象から除外されています。

起動を確認します。

```sh
curl --fail-with-body http://127.0.0.1:8080/version
```

## 設定

設定の雛形は [config.example.toml](config.example.toml) を参照してください。

| 設定 | 内容 |
| --- | --- |
| `server.host`, `server.port` | 待ち受け先。既定値は `127.0.0.1:8080`。 |
| `agent.max_llm_calls` | Explorerを含む各エージェント実行のLLM呼び出し上限。既定値は `30`。正の値が必要。 |
| `agent.model` | 全ロールの既定モデル ID。 |
| `agent.<role>.model` | `planner`、`research`、`gap_judger`、`explorer`、`synthesizer` のモデル ID を上書き。 |
| `agent.<role>.system_prompt` | システムプロンプトを文字列で指定。省略時は `src/assets/` の組み込みファイルを使用。 |
| `models[].id` | ロールから参照する、一意のモデル ID。 |
| `models[].provider` | 設定済みプロバイダーの ID。 |
| `models[].name` | プロバイダーに送信するモデル名。 |
| `models[].max_concurrency` | このモデル ID の最大同時リクエスト数。既定値は `1`。正の値が必要。 |
| `providers[].id` | 一意のプロバイダー ID。 |
| `providers[].type` | `OpenAI` または `Anthropic`。大文字・小文字を区別。 |
| `providers[].api_key_env` | 認証に使う環境変数名。 |
| `providers[].endpoint` | API ベース URL の上書き。省略可能。 |
| `tools.web_search.searxng.endpoint` | SearXNG の検索 URL 全体。クライアントが `format=json` と `q` を追加。 |

モデルの選択では、ロールごとの指定、`agent.model` の順で優先します。両方が未設定の場合、`gap_judger` と `explorer` は research ロールで解決されたモデルを使います。planner、research、synthesizer には、ロールごとのモデルか `agent.model` の指定が必要です。

Web取得の上限は `[tools.web_fetch]`、検索の上限は `[tools.web_search.searxng]` で設定します。`connect_timeout_secs` は既定で10秒、`request_timeout_secs` は60秒、`max_body_bytes` は2097152バイト（2 MiB）です。すべて正の値が必要です。fetchの60秒にはリダイレクト先の取得も含みます。

## HTTP API

| メソッド | パス | レスポンス |
| --- | --- | --- |
| `GET` | `/version` | パッケージのバージョンと対応 API バージョン。 |
| `POST` | `/v1/deep/research/plan` | JSON 形式の調査計画。 |
| `POST` | `/v1/deep/research` | SSE 形式の調査イベント。 |

### 計画の作成

```sh
curl --fail-with-body http://127.0.0.1:8080/v1/deep/research/plan \
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

計画を修正する場合は、同じエンドポイントに `prompt` と、省略可能な `previous_plan` を送信します。`previous_plan` には計画オブジェクト全体を指定します。レスポンスには修正後の計画全体が返ります。計画作成に失敗した場合は HTTP `500` を返します。

### 調査の実行

保存した計画を `plan` フィールドで包み、送信します。

```sh
jq '{plan: .}' plan.json > research-request.json
curl --fail-with-body -N http://127.0.0.1:8080/v1/deep/research \
  -H 'Content-Type: application/json' \
  --data-binary @research-request.json
```

SSE の各データフレームには、`model`、`phase`、`data` を持つ JSON が入ります。`model` は設定したモデル ID です。フェーズは `Researching`、`GapJudging`、`ResearchStepCompleted`、`Synthesizing`、`Synthesized`、`Failed` です。並行して実行する調査項目のイベントは混在する場合があります。

調査項目の完了イベントの例です。

```text
data: {"model":"default","phase":"ResearchStepCompleted","data":{"findings":[{"question":"主要な技術の利点と制約は？","answer":"調査結果...","status":"supported","references":[{"source":"https://example.com/source","content":"根拠となる情報..."}]}],"limitations":[]}}

```

イベントを待つ間は、15 秒ごとに `: keep-alive` コメントを送信します。正常終了時は `Synthesized` を出力します。調査や統合に失敗した場合は、`data.error` を持つ `Failed` を出力してストリームを閉じます。HTTP ステータスが `200` でも、クライアントはイベント内の失敗を確認する必要があります。接続を切断すると、その調査タスクはキャンセルされます。

`Synthesized.data` には `title`、`summary`、`sections`（`heading`、`content`、`sources`）、`limitations` を持つ最終レポートが入ります。各ステップの契約と旧形式からの移行方法は [ステップ間のデータ契約](docs/stage-contracts.md) を参照してください。旧形式の計画と調査結果は互換性がありません。

## 現在の制約

- ページ取得は HTML などを含むレスポンスのテキストをそのまま返します。記事本文の抽出や JavaScript の実行は行いません。
- fetchは公開HTTP(S) URLだけを許可し、DNS解決後のIPとリダイレクト先も検証します。内部・ループバック・リンクローカル・予約済みIPは拒否し、環境変数のプロキシは使いません。設定したLLMとSearXNGの接続先には内部URLを使用できます。
- SearXNG結果の `url`、`title`、`score` は必須ですが、公開日時の `publishedDate` は省略・nullを許容します。ツール出力の `published_date` はUTC日時またはnullになります。
- 各エージェント実行は `agent.max_llm_calls` で制限します。調査・レビューの10回制限は別に維持します。Web取得の時間・本文サイズ超過はツールエラーとしてエージェントに返します。

## 開発

```sh
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
```

| クレート | 役割 |
| --- | --- |
| `deep-research-agent` | 実行ファイル、設定、エージェントの組み立て。 |
| `deep-research-api` | Actix Web のエンドポイントと SSE レスポンス。 |
| `deep-research-orchestrator` | 計画、調査の制御、不足の確認、結果の統合。 |
| `deep-research-react-agent` | ReAct の実行とエージェントイベント。 |
| `deep-research-agent-tools` | ツールとして呼び出せる Explorer エージェント。 |
| `deep-research-tools` | ツールのインターフェース、検索、ページ取得、マーカーツール。 |
| `deep-research-runner` | LLM リクエストの実行。 |
| `deep-research-arbiter` | モデルの同時実行数の制御。 |

## ライセンス

[Apache License 2.0](LICENSE)。
