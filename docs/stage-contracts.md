# ステップ間のデータ契約

各エージェントは `submit` の引数として以下の JSON を返します。フィールドはすべて必須です。該当する要素がなければ配列を `[]` にし、未定義のフィールドは追加しません。

ローカルLLMが生成する構造を単純に保つため、固定キーのオブジェクト、配列、文字列、真偽値、小さな列挙型だけを使います。任意フィールド、null を含む出力型、型の分岐、モデルが採番するIDは使いません。ツールの JSON Schema は入れ子の型をインライン化し、`$ref` を辿る必要をなくしています。質問と章の対応は、計画内の文字列をそのままコピーし、配列の順序を保つことで表します。

## Plan → Research / Synthesizer

```json
{
  "research_plans": [
    {
      "goal": "主要な蓄電技術の特徴を比較する",
      "scope": "定置型蓄電。車載用途は除く",
      "questions": ["主要な技術にはどのような利点と制約があるか？"]
    }
  ],
  "report_plan": {
    "goal": "定置型蓄電の技術選定に役立つ比較レポート",
    "sections": [
      {"heading": "技術の比較", "focus": "利点と制約を比較する"}
    ]
  }
}
```

調査ステップは独立して実行されるため、それぞれが必要な範囲と質問を持ちます。調査ステップ、各質問の配列、レポートの章は空にできません。質問と章見出しはそれぞれの配列内で重複させません。再計画でも差分ではなく計画全体を返します。

## Research → Gap Judge / Synthesizer

```json
{
  "findings": [
    {
      "question": "主要な技術にはどのような利点と制約があるか？",
      "answer": "確認できた利点と制約を根拠に基づいて記載する",
      "status": "partial",
      "references": [
        {"source": "https://example.com/source", "content": "この回答を支える資料の抜粋または忠実な要約"}
      ]
    }
  ],
  "limitations": ["一部の技術について比較可能な資料が不足している"]
}
```

`findings` は計画の質問と同じ件数・順序にします。各項目の `question` は対応する質問を完全一致でコピーします。根拠は回答ごとに格納するため、回答と資料の対応が失われません。`references` は実際に確認した資料だけを含めます。

| status | 意味 |
| --- | --- |
| `supported` | 根拠のある完全な回答。少なくとも一つの参照が必要 |
| `partial` | 一部を回答できたが不足がある |
| `unanswered` | 回答を確立できていない。`answer` に何が不明かを書く |

## Gap Judge → 再調査

```json
{
  "approved": false,
  "gaps": [
    {
      "question": "主要な技術にはどのような利点と制約があるか？",
      "kind": "insufficient_evidence",
      "reason": "一部の技術を裏付ける一次資料が不足している",
      "next_action": "該当技術の一次資料を確認し、利点と制約の根拠を追加する"
    }
  ]
}
```

`kind` は `missing_answer`、`insufficient_evidence`、`conflicting_evidence` のいずれかです。`question` は計画内の質問をコピーします。承認の場合は `{"approved": true, "gaps": []}` を返します。不承認では少なくとも一つの不足と具体的な調査指示が必要です。

再調査の入力には `research_plan`、`previous_research_output`、`previous_gap_analysis` を渡します。初回は後ろの二つが `null` です。再調査では前回の根拠ある回答を維持し、不足を解消して全質問の結果を再提出します。試行上限まで承認されなければ `Failed` になります。

## Research → Synthesizer

統合の入力は `report_plan` と `research_outputs` です。`research_outputs` の各要素は `{"research_plan": ..., "research_output": ...}` として、調査の範囲と結果を組にして渡します。並行調査の完了順にかかわらず、計画内の順序を維持します。

## Synthesizer → 最終結果

```json
{
  "title": "定置型蓄電技術の比較",
  "summary": "主要な結論を記載する",
  "sections": [
    {
      "heading": "技術の比較",
      "content": "計画の focus に沿ったレポート本文。Markdown も使用可能",
      "sources": ["https://example.com/source"]
    }
  ],
  "limitations": ["残っている不確実性を記載する"]
}
```

章は `report_plan.sections` と同じ件数・順序とし、見出しをコピーします。`sources` は調査結果内の根拠の `source` を完全一致でコピーします。未知の資料を引用した結果は拒否します。最終レポート全体が SSE の `Synthesized.data` に格納されます。

## 検証と移行

計画の空欄・空配列・重複、質問の件数や対応、根拠のない `supported`、承認と不足リストの矛盾、具体的指示のない不足、計画と異なる章、未知の引用元を実行時に検証します。これらの検証は資料の真偽や引用の意味的な適切さを保証するものではなく、内容の充足性は gap judge が評価します。LLMの出力の解析・検証に失敗した場合は、計画APIでは HTTP 500、調査中は `Failed` を返します。外部から渡された無効な計画は調査APIで HTTP 400 になります。

この変更は従来の JSON と互換性がありません。保存済みの `goal` だけの計画は `scope`、`questions`、`report_plan.sections` を追加するか、計画APIで再生成してください。SSE利用側は `research_step_result` とステップ直下の `references` を `findings` と回答ごとの `references` に変更し、空だった `Synthesized.data` からレポート本文を読むよう更新してください。独自のシステムプロンプトを設定している場合も、新しい必須フィールドに合わせてください。
