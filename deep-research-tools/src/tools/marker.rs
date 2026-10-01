use crate::DeepResearchTool;
use async_trait::async_trait;
use genai::chat::{ToolConfig, ToolName};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct MarkerTool<A> {
    name: ToolName,
    description: Option<String>,
    strict: Option<bool>,
    config: Option<ToolConfig>,
    _phantom: std::marker::PhantomData<A>,
}

impl<A> MarkerTool<A>
where
    A: JsonSchema + Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
{
    pub fn new(
        name: ToolName,
        description: Option<String>,
        strict: Option<bool>,
        config: Option<ToolConfig>,
    ) -> Self {
        Self {
            name,
            description,
            strict,
            config,
            _phantom: std::marker::PhantomData,
        }
    }
}

#[derive(JsonSchema, Serialize, Deserialize)]
pub struct MarkerCalled;

#[async_trait]
impl<A> DeepResearchTool for MarkerTool<A>
where
    A: JsonSchema + Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
{
    type Args = A;
    type Output = MarkerCalled;

    fn name(&self) -> ToolName {
        self.name.clone()
    }

    fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    fn strict(&self) -> Option<bool> {
        self.strict
    }

    fn config(&self) -> Option<ToolConfig> {
        self.config.clone()
    }

    async fn call(&self, _args: Self::Args) -> anyhow::Result<Self::Output> {
        Ok(MarkerCalled)
    }
}
