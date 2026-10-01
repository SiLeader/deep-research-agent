use crate::DeepResearchTool;
use async_trait::async_trait;
use genai::chat::Tool;
use serde::Serialize;
use serde::de::DeserializeOwned;

pub(crate) struct Wrapped<T> {
    inner: T,
}

impl<T, A, O> Wrapped<T>
where
    T: DeepResearchTool<Args = A, Output = O> + 'static,
    A: Serialize + DeserializeOwned + 'static,
    O: Serialize + DeserializeOwned + 'static,
{
    pub(crate) fn new(inner: T) -> Self {
        Self { inner }
    }
}

#[async_trait]
pub trait WrappedTool: Send + Sync {
    fn tool_description(&self) -> anyhow::Result<Tool>;

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<serde_json::Value>;
}

#[async_trait]
impl<T> WrappedTool for Wrapped<T>
where
    T: DeepResearchTool + 'static,
    T::Args: serde::Serialize + DeserializeOwned + 'static,
    T::Output: serde::Serialize + DeserializeOwned + 'static,
{
    fn tool_description(&self) -> anyhow::Result<Tool> {
        Ok(Tool {
            name: self.inner.name(),
            description: self.inner.description().map(ToString::to_string),
            schema: self.inner.schema()?,
            strict: self.inner.strict(),
            config: self.inner.config(),
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let args: T::Args = serde_json::from_value(args)?;
        let output = self.inner.call(args).await?;
        let output = serde_json::to_value(output)?;
        Ok(output)
    }
}
