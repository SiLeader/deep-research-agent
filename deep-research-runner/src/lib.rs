use deep_research_arbiter::AgentConcurrencyArbiter;
use genai::Client;
use genai::chat::{ChatMessage, ChatOptions, ChatRequest, ChatResponse, Tool};
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// Upper bound for a server-requested `Retry-After` delay.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(300);

tokio::task_local! {
    static CALL_BUDGET: Arc<CallBudget>;
}

/// LLM request budget shared by every runner invoked inside [`with_call_budget`].
/// Each attempt, including retries, consumes one call.
pub struct CallBudget {
    limit: usize,
    used: AtomicUsize,
}

impl CallBudget {
    pub fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            limit,
            used: AtomicUsize::new(0),
        })
    }

    fn consume(&self) -> anyhow::Result<()> {
        let used = self.used.fetch_add(1, Ordering::SeqCst);
        anyhow::ensure!(
            used < self.limit,
            "LLM call budget exhausted ({} calls)",
            self.limit
        );
        Ok(())
    }
}

/// Run `future` with `budget` applied to all LLM calls made by the current task.
/// Spawned tasks do not inherit the budget; wrap their futures as well.
pub async fn with_call_budget<F: Future>(budget: Arc<CallBudget>, future: F) -> F::Output {
    CALL_BUDGET.scope(budget, future).await
}

/// Honor the server's requested delay; retrying earlier would only repeat the
/// throttled response and spend the call budget.
fn retry_delay(retry_after: Option<Duration>, backoff: Duration) -> Duration {
    match retry_after {
        Some(delay) => delay.min(MAX_RETRY_AFTER),
        None => backoff,
    }
}

fn consume_budget() -> anyhow::Result<()> {
    CALL_BUDGET
        .try_with(|budget| budget.consume())
        .unwrap_or(Ok(()))
}

#[derive(Clone)]
pub struct OneshotRunner {
    model: String,
    client: Client,
    arbiter: Arc<dyn AgentConcurrencyArbiter>,
    options: ChatOptions,
    request_timeout: Duration,
    max_retries: u32,
}

enum AttemptError {
    Timeout,
    Genai(genai::Error),
    Other(anyhow::Error),
}

impl AttemptError {
    /// Retry timeouts, connection failures, and throttling or server errors.
    fn retry_after(&self) -> Option<Option<Duration>> {
        use genai::webc::Error as WebError;
        let retryable_status = |status: u16| matches!(status, 408 | 409 | 425 | 429 | 500..=599);
        match self {
            AttemptError::Timeout => Some(None),
            AttemptError::Genai(genai::Error::WebStream { .. }) => Some(None),
            AttemptError::Genai(genai::Error::HttpError { status, .. }) => {
                retryable_status(status.as_u16()).then_some(None)
            }
            AttemptError::Genai(
                genai::Error::WebModelCall { webc_error, .. }
                | genai::Error::WebAdapterCall { webc_error, .. },
            ) => match webc_error {
                WebError::ResponseFailedStatus {
                    status, headers, ..
                } => retryable_status(status.as_u16()).then(|| {
                    headers
                        .get("retry-after")
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.trim().parse::<u64>().ok())
                        .map(Duration::from_secs)
                }),
                WebError::Reqwest(error) => {
                    (error.is_timeout() || error.is_connect() || error.is_request()).then_some(None)
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn into_anyhow(self, model: &str) -> anyhow::Error {
        match self {
            AttemptError::Timeout => anyhow::anyhow!("LLM request timed out for model: {model}"),
            AttemptError::Genai(error) => error.into(),
            AttemptError::Other(error) => error,
        }
    }
}

impl OneshotRunner {
    pub fn new(
        model: String,
        client: Client,
        arbiter: Arc<dyn AgentConcurrencyArbiter>,
        options: ChatOptions,
    ) -> Self {
        Self {
            model,
            client,
            arbiter,
            options,
            request_timeout: Duration::from_secs(120),
            max_retries: 0,
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Deadline for one chat request, starting after the model permit is acquired.
    pub fn with_request_timeout(mut self, timeout_secs: u64) -> anyhow::Result<Self> {
        anyhow::ensure!(
            timeout_secs > 0,
            "LLM request_timeout_secs must be positive"
        );
        self.request_timeout = Duration::from_secs(timeout_secs);
        Ok(self)
    }

    /// Retry transient failures with exponential backoff. The model permit is
    /// released while waiting.
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    pub async fn run(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<Tool>,
    ) -> anyhow::Result<ChatResponse> {
        let request = ChatRequest::new(messages).with_tools(tools);
        let mut backoff = INITIAL_BACKOFF;
        let mut attempt = 0;
        loop {
            consume_budget()?;
            let error = match self.attempt(request.clone()).await {
                Ok(response) => return Ok(response),
                Err(error) => error,
            };
            let Some(retry_after) = error.retry_after().filter(|_| attempt < self.max_retries)
            else {
                return Err(error.into_anyhow(&self.model));
            };
            attempt += 1;
            let delay = retry_delay(retry_after, backoff);
            tracing::warn!(
                model = %self.model,
                attempt,
                delay_ms = delay.as_millis() as u64,
                error = %error.into_anyhow(&self.model),
                "Retrying LLM request"
            );
            tokio::time::sleep(delay).await;
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    }

    async fn attempt(&self, request: ChatRequest) -> Result<ChatResponse, AttemptError> {
        let _guard = self
            .arbiter
            .acquire(&self.model)
            .await
            .map_err(AttemptError::Other)?;
        let request = self
            .client
            .exec_chat(&self.model, request, Some(&self.options));
        tokio::time::timeout(self.request_timeout, request)
            .await
            .map_err(|_| AttemptError::Timeout)?
            .map_err(AttemptError::Genai)
    }
}

#[cfg(test)]
mod tests;
