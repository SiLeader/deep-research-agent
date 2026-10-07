//! In-memory search over fetched pages and search-result text.
//!
//! Scores are larger-is-better: BM25 relevance for lexical search, RRF scores
//! for hybrid search, and provider relevance scores when reranking is enabled.
use anyhow::ensure;
use deep_research_reranker::DeepResearchReranker;
mod vector;
use sqlx::{Connection, Executor, SqliteConnection};
use std::collections::BTreeMap;
use text_splitter::{Characters, TextSplitter};
use tokio::sync::Mutex;
use vector::VectorDb;

pub struct FetchedDb {
    state: Mutex<State>,
    splitter: TextSplitter<Characters>,
    embedder: Option<Embedder>,
    reranker: Option<Reranker>,
}

struct State {
    sqlite: SqliteConnection,
    emvedb: Option<VectorDb>,
}

#[derive(Clone)]
pub struct Embedder {
    model: String,
    embed: genai::Client,
    request_timeout: Option<std::time::Duration>,
    arbiter: Option<std::sync::Arc<dyn deep_research_arbiter::AgentConcurrencyArbiter>>,
}

impl Embedder {
    pub fn new(model: impl Into<String>, client: genai::Client) -> Self {
        Self {
            model: model.into(),
            embed: client,
            request_timeout: None,
            arbiter: None,
        }
    }

    /// Share the model concurrency budget with chat calls and other explorations.
    pub fn with_arbiter(
        mut self,
        arbiter: std::sync::Arc<dyn deep_research_arbiter::AgentConcurrencyArbiter>,
    ) -> Self {
        self.arbiter = Some(arbiter);
        self
    }

    pub fn with_request_timeout(mut self, timeout_secs: u64) -> anyhow::Result<Self> {
        ensure!(
            timeout_secs > 0,
            "embedding request_timeout_secs must be positive"
        );
        self.request_timeout = Some(std::time::Duration::from_secs(timeout_secs));
        Ok(self)
    }

    async fn vectors(&self, texts: Vec<String>) -> anyhow::Result<Vec<Vec<f32>>> {
        let _permit = match &self.arbiter {
            Some(arbiter) => Some(arbiter.acquire(&self.model).await?),
            None => None,
        };
        let count = texts.len();
        let request = self.embed.embed_batch(&self.model, texts, None);
        let response = match self.request_timeout {
            Some(timeout) => tokio::time::timeout(timeout, request)
                .await
                .map_err(|_| anyhow::anyhow!("embedding request timed out"))??,
            None => request.await?,
        };
        ensure!(
            response.embeddings.len() == count,
            "embedding count mismatch"
        );
        let mut vectors = vec![None; count];
        for embedding in response.embeddings {
            ensure!(embedding.index < count, "embedding index out of range");
            ensure!(
                vectors[embedding.index].is_none(),
                "duplicate embedding index"
            );
            validate_vector(&embedding.vector, None)?;
            vectors[embedding.index] = Some(embedding.vector);
        }
        Ok(vectors.into_iter().map(Option::unwrap).collect())
    }
}

#[derive(Clone)]
pub struct Reranker {
    model: String,
    reranker: DeepResearchReranker,
}

impl Reranker {
    pub fn new(model: impl Into<String>, reranker: DeepResearchReranker) -> Self {
        Self {
            model: model.into(),
            reranker,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FoundData {
    pub url: String,
    pub content: String,
    pub score: f32,
}

impl FetchedDb {
    pub async fn new(
        chunk_size: usize,
        embedder: Option<Embedder>,
        reranker: Option<Reranker>,
    ) -> anyhow::Result<Self> {
        ensure!(chunk_size > 0, "chunk_size must be positive");
        let mut sqlite = SqliteConnection::connect(":memory:").await?;
        sqlite.execute("CREATE VIRTUAL TABLE chunks USING fts5(url UNINDEXED, content, tokenize='unicode61')").await?;
        Ok(Self {
            state: Mutex::new(State {
                sqlite,
                emvedb: None,
            }),
            splitter: TextSplitter::new(chunk_size),
            embedder,
            reranker,
        })
    }

    pub async fn add_html(&self, url: &str, html: &str) -> anyhow::Result<()> {
        self.add_text(url, &htmd::convert(html)?).await
    }

    /// Append plain text or Markdown, including snippets returned by web search.
    /// Multiple chunks (including identical text at different URLs) remain distinct.
    pub async fn add_text(&self, url: &str, text: &str) -> anyhow::Result<()> {
        let chunks: Vec<String> = self
            .splitter
            .chunks(text)
            .filter(|chunk| !chunk.trim().is_empty())
            .map(str::to_owned)
            .collect();
        if chunks.is_empty() {
            return Ok(());
        }
        let vectors = match &self.embedder {
            Some(embedder) => Some(embedder.vectors(chunks.clone()).await?),
            None => None,
        };
        let mut state = self.state.lock().await;
        if let Some(vectors) = &vectors {
            let dimension = vectors[0].len();
            for vector in vectors {
                validate_vector(vector, Some(dimension))?;
            }
            if let Some(db) = &state.emvedb {
                ensure!(
                    db.dimension() as usize == dimension,
                    "embedding dimension changed"
                );
            } else {
                state.emvedb = Some(VectorDb::create(dimension as u32)?);
            }
        }
        let State { sqlite, emvedb } = &mut *state;
        let mut tx = sqlite.begin().await?;
        // Roll back vector writes on errors or cancellation along with the SQL
        // transaction, so failed batches cannot appear in subsequent searches.
        let mut pending = PendingVectors {
            db: emvedb.as_ref(),
            ids: Vec::new(),
        };
        for (index, content) in chunks.iter().enumerate() {
            let id = sqlx::query("INSERT INTO chunks(url, content) VALUES (?, ?)")
                .bind(url)
                .bind(content)
                .execute(&mut *tx)
                .await?
                .last_insert_rowid();
            if let (Some(db), Some(vectors)) = (pending.db, &vectors) {
                pending.ids.push(id as u64);
                db.put(id as u64, &vectors[index])?;
            }
        }
        tx.commit().await?;
        pending.ids.clear();
        Ok(())
    }

    /// Retrieve an expanded candidate set before optional fusion and reranking.
    /// Queries are treated as literal words joined with OR, not FTS operators.
    pub async fn search(&self, query: &str, top_k: usize) -> anyhow::Result<Vec<FoundData>> {
        if top_k == 0 || query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let mut state = self.state.lock().await;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM chunks")
            .fetch_one(&mut state.sqlite)
            .await?;
        if count == 0 {
            return Ok(Vec::new());
        }
        let limit = if self.embedder.is_some() || self.reranker.is_some() {
            top_k.saturating_mul(4).max(20)
        } else {
            top_k
        }
        .min(count as usize);
        let fts_query = literal_query(query);
        let mut candidates = BTreeMap::new();
        let mut lexical = Vec::new();
        if !fts_query.is_empty() {
            let rows: Vec<(i64, String, String, f64)> = sqlx::query_as(
                "SELECT rowid, url, content, bm25(chunks) FROM chunks WHERE chunks MATCH ? ORDER BY bm25(chunks), rowid LIMIT ?"
            ).bind(fts_query).bind(limit as i64).fetch_all(&mut state.sqlite).await?;
            for (id, url, content, score) in rows {
                lexical.push(id);
                candidates.insert(
                    id,
                    FoundData {
                        url,
                        content,
                        score: -score as f32,
                    },
                );
            }
        }
        let mut vector_ids = Vec::new();
        if let (Some(embedder), Some(db)) = (&self.embedder, &state.emvedb) {
            let vectors = embedder.vectors(vec![query.to_owned()]).await?;
            validate_vector(&vectors[0], Some(db.dimension() as usize))?;
            let results = db.search(&vectors[0], limit)?;
            for result in results {
                let id = result as i64;
                let (url, content): (String, String) =
                    sqlx::query_as("SELECT url, content FROM chunks WHERE rowid = ?")
                        .bind(id)
                        .fetch_one(&mut state.sqlite)
                        .await?;
                vector_ids.push(id);
                candidates.entry(id).or_insert(FoundData {
                    url,
                    content,
                    score: 0.0,
                });
            }
            let scores = rrf(&lexical, &vector_ids);
            for (id, data) in &mut candidates {
                data.score = scores[id];
            }
        }
        drop(state);
        // BTreeMap gives deterministic row-id order for tied scores.
        let mut found: Vec<_> = candidates.into_values().collect();
        found.sort_by(|a, b| b.score.total_cmp(&a.score));
        found.truncate(limit);
        if let Some(reranker) = &self.reranker {
            if found.is_empty() {
                return Ok(found);
            }
            let results = reranker
                .reranker
                .rerank_indices(
                    reranker.model.clone(),
                    query.to_owned(),
                    found.iter().map(|data| data.content.clone()).collect(),
                    top_k,
                )
                .await?;
            found = results
                .into_iter()
                .map(|(index, score)| {
                    let mut data = found[index].clone();
                    data.score = score;
                    data
                })
                .collect();
            found.sort_by(|a, b| b.score.total_cmp(&a.score));
        }
        found.truncate(top_k);
        Ok(found)
    }
}

struct PendingVectors<'a> {
    db: Option<&'a VectorDb>,
    ids: Vec<u64>,
}

impl Drop for PendingVectors<'_> {
    fn drop(&mut self) {
        if let Some(db) = self.db {
            for &id in &self.ids {
                let _ = db.delete(id);
            }
        }
    }
}

fn validate_vector(vector: &[f32], dimension: Option<usize>) -> anyhow::Result<()> {
    ensure!(
        (1..=65535).contains(&vector.len()),
        "invalid embedding dimension"
    );
    ensure!(
        dimension.is_none_or(|dimension| vector.len() == dimension),
        "embedding dimension mismatch"
    );
    ensure!(
        vector.iter().all(|value| value.is_finite()),
        "non-finite embedding value"
    );
    ensure!(
        vector.iter().any(|value| *value != 0.0),
        "zero embedding vector"
    );
    Ok(())
}

fn literal_query(query: &str) -> String {
    query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| format!("\"{word}\""))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn rrf(lexical: &[i64], vectors: &[i64]) -> BTreeMap<i64, f32> {
    let mut scores = BTreeMap::new();
    for ranking in [lexical, vectors] {
        for (rank, &id) in ranking.iter().enumerate() {
            *scores.entry(id).or_default() += 1.0 / (60.0 + (rank + 1) as f32);
        }
    }
    scores
}

#[cfg(test)]
mod tests {
    use super::*;
    use genai::{
        Client, ModelIden, ServiceTarget,
        adapter::AdapterKind,
        resolver::{AuthData, Endpoint},
    };
    use serde_json::{Value, json};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn fixture(responses: Vec<Value>) -> (String, tokio::task::JoinHandle<Vec<Value>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}/", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for body in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut buf = [0; 4096];
                    let n = socket.read(&mut buf).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            requests.push(
                                serde_json::from_slice(&request[end + 4..end + 4 + length])
                                    .unwrap(),
                            );
                            break;
                        }
                    }
                }
                let body = body.to_string();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
            requests
        });
        (origin, task)
    }

    fn embedder(origin: String) -> Embedder {
        let client = Client::builder()
            .with_service_target_resolver_fn(move |_: ServiceTarget| {
                Ok(ServiceTarget {
                    model: ModelIden::new(AdapterKind::OpenAI, "test-embed"),
                    auth: AuthData::from_single("test"),
                    endpoint: Endpoint::from_owned(origin.clone()),
                })
            })
            .build();
        Embedder::new("test-embed", client)
    }

    fn embedding(vector: Vec<f32>) -> Value {
        json!({"object": "list", "model": "test-embed", "data": [{"object": "embedding", "index": 0, "embedding": vector}], "usage": {"prompt_tokens": 1, "total_tokens": 1}})
    }

    #[tokio::test]
    async fn supports_all_four_search_modes() {
        for use_embedder in [false, true] {
            for use_reranker in [false, true] {
                let (origin, embed_task) = fixture(if use_embedder {
                    vec![
                        embedding(vec![1.0, 0.0]),
                        embedding(vec![0.0, 1.0]),
                        embedding(vec![0.0, 1.0]),
                    ]
                } else {
                    vec![]
                })
                .await;
                let (rerank_origin, rerank_task) = fixture(if use_reranker {
                    vec![json!({"id": "test", "results": [
                        {"index": if use_embedder { 1 } else { 0 }, "relevance_score": 0.95}
                    ]})]
                } else {
                    vec![]
                })
                .await;
                let db = FetchedDb::new(
                    1024,
                    use_embedder.then(|| embedder(origin)),
                    use_reranker.then(|| {
                        Reranker::new(
                            "test-rerank",
                            DeepResearchReranker::new(rerank_origin.trim_end_matches('/').into()),
                        )
                    }),
                )
                .await
                .unwrap();
                db.add_html("https://apple.test", "<p>apple orchard</p>")
                    .await
                    .unwrap();
                db.add_text("https://banana.test", "banana plantation")
                    .await
                    .unwrap();
                let found = db.search("apple", 1).await.unwrap();
                assert_eq!(found.len(), 1);
                assert_eq!(
                    found[0].url,
                    if use_embedder && use_reranker {
                        "https://banana.test"
                    } else {
                        "https://apple.test"
                    }
                );
                if use_reranker {
                    assert_eq!(found[0].score, 0.95);
                } else if use_embedder {
                    assert!((found[0].score - (1.0 / 61.0 + 1.0 / 62.0)).abs() < 1e-6);
                } else {
                    assert!(found[0].score > 0.0);
                }
                let requests = embed_task.await.unwrap();
                assert_eq!(requests.len(), if use_embedder { 3 } else { 0 });
                let requests = rerank_task.await.unwrap();
                if use_reranker {
                    assert_eq!(requests[0]["query"], "apple");
                    assert_eq!(
                        requests[0]["documents"].as_array().unwrap().len(),
                        if use_embedder { 2 } else { 1 }
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn lexical_search_handles_empty_input_html_and_literal_queries() {
        assert!(FetchedDb::new(0, None, None).await.is_err());
        let db = FetchedDb::new(30, None, None).await.unwrap();
        assert!(db.search("apple", 10).await.unwrap().is_empty());
        db.add_html(
            "https://a.test",
            "<h1>Apple</h1><p>Orchard grows apples and pears.</p>",
        )
        .await
        .unwrap();
        db.add_text("https://b.test", "Bananas grow in plantations.")
            .await
            .unwrap();
        db.add_text("https://empty.test", "  \n ").await.unwrap();
        assert!(db.search("", 10).await.unwrap().is_empty());
        assert!(db.search("apple", 0).await.unwrap().is_empty());
        assert!(db.search("\" () : *", 10).await.unwrap().is_empty());
        assert!(db.search("missing", 10).await.unwrap().is_empty());
        let found = db.search("\"apple\" OR banana*", 10).await.unwrap();
        assert!(found.iter().all(|data| data.url == "https://a.test"));
        assert!(found.iter().all(|data| !data.content.contains("<h1>")));
    }

    #[test]
    fn fusion_accumulates_shared_candidates() {
        let scores = rrf(&[1, 2], &[2, 3]);
        assert!(scores[&2] > scores[&1]);
        assert_eq!(scores[&1], 1.0 / 61.0);
        assert_eq!(scores[&3], 1.0 / 62.0);
    }

    #[tokio::test]
    async fn invalid_embeddings_leave_the_index_unchanged() {
        let (origin, task) = fixture(vec![embedding(vec![0.0, 0.0])]).await;
        let db = FetchedDb::new(100, Some(embedder(origin)), None)
            .await
            .unwrap();
        assert!(db.add_text("https://bad.test", "apple").await.is_err());
        assert!(db.search("apple", 10).await.unwrap().is_empty());
        assert!(db.state.lock().await.emvedb.is_none());
        task.await.unwrap();
    }
    #[tokio::test]
    async fn semantic_search_respects_batch_indices_and_rejects_dimension_changes() {
        let batch = json!({"object": "list", "model": "test-embed", "data": [
            {"object": "embedding", "index": 1, "embedding": [0.0, 1.0]},
            {"object": "embedding", "index": 0, "embedding": [1.0, 0.0]}
        ], "usage": {"prompt_tokens": 2, "total_tokens": 2}});
        let (origin, task) = fixture(vec![
            batch,
            embedding(vec![0.0, 1.0]),
            embedding(vec![1.0, 0.0, 0.0]),
            embedding(vec![0.0, 1.0]),
        ])
        .await;
        let db = FetchedDb::new(6, Some(embedder(origin)), None)
            .await
            .unwrap();
        db.add_text("https://fruit.test", "apple\n\nbanana")
            .await
            .unwrap();
        let found = db.search("fruit", 1).await.unwrap();
        assert_eq!(found[0].content, "banana");
        assert!(db.add_text("https://bad.test", "pear").await.is_err());
        let found = db.search("fruit", 10).await.unwrap();
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|data| data.url == "https://fruit.test"));
        let requests = task.await.unwrap();
        assert_eq!(requests[0]["input"], json!(["apple", "banana"]));
    }

    #[tokio::test]
    async fn reranking_preserves_urls_for_identical_content_and_validates_indices() {
        let (origin, task) = fixture(vec![
            json!({"id": "test", "results": [{"index": 1, "relevance_score": 0.9}]}),
            json!({"id": "test", "results": [{"index": 2, "relevance_score": 0.9}]}),
            json!({"id": "test", "results": [
                {"index": 0, "relevance_score": 0.9}, {"index": 0, "relevance_score": 0.8}
            ]}),
        ])
        .await;
        let db = FetchedDb::new(
            100,
            None,
            Some(Reranker::new(
                "test-rerank",
                DeepResearchReranker::new(origin.trim_end_matches('/').into()),
            )),
        )
        .await
        .unwrap();
        db.add_text("https://a.test", "apple").await.unwrap();
        db.add_text("https://b.test", "apple").await.unwrap();
        assert_eq!(
            db.search("apple", 1).await.unwrap()[0].url,
            "https://b.test"
        );
        assert!(
            db.search("apple", 1)
                .await
                .unwrap_err()
                .to_string()
                .contains("out of range")
        );
        assert!(
            db.search("apple", 2)
                .await
                .unwrap_err()
                .to_string()
                .contains("duplicate")
        );
        task.await.unwrap();
    }
}
