pub mod tools;
mod wrap;

use crate::wrap::{Wrapped, WrappedTool};
use async_trait::async_trait;
use genai::chat::{Tool, ToolConfig, ToolName};
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::sync::Arc;

#[async_trait]
pub trait DeepResearchTool: Send + Sync + Clone {
    type Args: JsonSchema + Serialize + DeserializeOwned;
    type Output: JsonSchema + Serialize + DeserializeOwned;

    fn name(&self) -> ToolName;

    fn description(&self) -> Option<&str> {
        None
    }

    fn strict(&self) -> Option<bool> {
        Some(true)
    }

    fn schema(&self) -> anyhow::Result<Option<serde_json::Value>> {
        let mut generator = schemars::SchemaGenerator::default();
        Ok(Some(serde_json::to_value(
            <Self as DeepResearchTool>::Args::json_schema(&mut generator),
        )?))
    }

    fn config(&self) -> Option<ToolConfig> {
        None
    }

    async fn call(&self, args: Self::Args) -> anyhow::Result<Self::Output>;
}

#[derive(Clone, Default)]
pub struct DeepResearchTools {
    tools: HashMap<String, Arc<dyn WrappedTool>>,
}

impl DeepResearchTools {
    pub fn add<T>(&mut self, tool: T)
    where
        T: DeepResearchTool + 'static,
    {
        let name = tool.name().to_string();
        self.tools.insert(name, Arc::new(Wrapped::new(tool)));
    }

    pub fn tools(&self) -> anyhow::Result<Vec<Tool>> {
        self.tools.values().map(|t| t.tool_description()).collect()
    }

    pub async fn call(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        let tool = self.tools.get(name);
        match tool {
            Some(tool) => Ok(Some(tool.call(args).await?)),
            None => Ok(None),
        }
    }
}
