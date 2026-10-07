pub mod fetched;
pub mod tools;
mod wrap;

use crate::wrap::{Wrapped, WrappedTool};
use async_trait::async_trait;
use genai::chat::{Tool, ToolConfig, ToolName};
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;
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
        // Inline small tool schemas so local models do not need to resolve $refs.
        let generator = schemars::generate::SchemaSettings::default()
            .with(|settings| settings.inline_subschemas = true)
            .into_generator();
        let mut schema = serde_json::to_value(generator.into_root_schema_for::<Self::Args>())?;
        if self.strict() == Some(true) {
            require_object_properties(&mut schema);
        }
        Ok(Some(schema))
    }

    fn config(&self) -> Option<ToolConfig> {
        None
    }

    async fn call(&self, args: Self::Args) -> anyhow::Result<Self::Output>;
}

// Strict providers require every property to be present. Preserve nullable
// Option schemas so callers can still request defaults by passing null.
fn require_object_properties(schema: &mut serde_json::Value) {
    match schema {
        serde_json::Value::Object(object) => {
            if object.get("type").and_then(serde_json::Value::as_str) == Some("object") {
                if let Some(properties) = object
                    .get("properties")
                    .and_then(serde_json::Value::as_object)
                {
                    let required: Vec<_> = properties.keys().cloned().collect();
                    object.insert("required".into(), serde_json::json!(required));
                }
                object.insert("additionalProperties".into(), serde_json::json!(false));
            }
            for value in object.values_mut() {
                require_object_properties(value);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                require_object_properties(item);
            }
        }
        _ => {}
    }
}

// Ordered by name so every registry presents tools identically, keeping
// provider prompt caches effective across fresh per-invocation registries.
#[derive(Clone, Default)]
pub struct DeepResearchTools {
    tools: BTreeMap<String, Arc<dyn WrappedTool>>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone, JsonSchema, Serialize, Deserialize)]
    struct TestArgs {
        value: i32,
    }

    #[derive(Clone)]
    struct TestTool {
        offset: i32,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl DeepResearchTool for TestTool {
        type Args = TestArgs;
        type Output = i32;

        fn name(&self) -> ToolName {
            ToolName::Custom("calculate".into())
        }

        fn description(&self) -> Option<&str> {
            Some("Add an offset to a nonnegative value")
        }

        async fn call(&self, args: TestArgs) -> anyhow::Result<i32> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            anyhow::ensure!(args.value >= 0, "value must be nonnegative");
            Ok(args.value + self.offset)
        }
    }

    fn registry() -> (DeepResearchTools, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut tools = DeepResearchTools::default();
        tools.add(TestTool {
            offset: 10,
            calls: calls.clone(),
        });
        (tools, calls)
    }

    #[tokio::test]
    async fn dispatches_typed_arguments_and_serializes_output() {
        let (tools, calls) = registry();
        assert_eq!(
            tools.call("calculate", json!({"value": 5})).await.unwrap(),
            Some(json!(15))
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn unknown_tool_returns_none_without_dispatching() {
        let (tools, calls) = registry();
        assert!(tools.call("missing", json!({})).await.unwrap().is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn malformed_arguments_are_rejected_before_dispatch() {
        let (tools, calls) = registry();
        for args in [json!({}), json!({"value": "wrong type"}), json!(null)] {
            assert!(tools.call("calculate", args).await.is_err());
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn tool_errors_are_propagated() {
        let (tools, calls) = registry();
        let error = tools
            .call("calculate", json!({"value": -1}))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "value must be nonnegative");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn adding_same_name_replaces_tool_and_clones_keep_previous_registry() {
        let (mut tools, calls) = registry();
        let cloned = tools.clone();
        tools.add(TestTool { offset: 20, calls });
        assert_eq!(tools.tools().unwrap().len(), 1);
        assert_eq!(
            tools.call("calculate", json!({"value": 5})).await.unwrap(),
            Some(json!(25))
        );
        assert_eq!(
            cloned.call("calculate", json!({"value": 5})).await.unwrap(),
            Some(json!(15))
        );
    }

    #[test]
    fn descriptions_include_argument_schema_and_metadata() {
        let (tools, _) = registry();
        let descriptions = tools.tools().unwrap();
        let tool = &descriptions[0];
        assert_eq!(tool.name.to_string(), "calculate");
        assert_eq!(
            tool.description.as_deref(),
            Some("Add an offset to a nonnegative value")
        );
        assert_eq!(tool.strict, Some(true));
        let schema = tool.schema.as_ref().unwrap();
        assert_eq!(schema["properties"]["value"]["type"], "integer");
        assert_eq!(schema["required"], json!(["value"]));
        assert!(DeepResearchTools::default().tools().unwrap().is_empty());
    }

    #[test]
    fn strict_schemas_require_nested_nullable_properties() {
        #[derive(JsonSchema)]
        struct Nested {
            #[allow(dead_code)]
            optional: Option<String>,
        }
        #[derive(JsonSchema)]
        struct Args {
            #[allow(dead_code)]
            items: Vec<Nested>,
        }
        let generator = schemars::generate::SchemaSettings::default()
            .with(|settings| settings.inline_subschemas = true)
            .into_generator();
        let mut schema = serde_json::to_value(generator.into_root_schema_for::<Args>()).unwrap();
        require_object_properties(&mut schema);
        assert_eq!(schema["required"], json!(["items"]));
        let nested = &schema["properties"]["items"]["items"];
        assert_eq!(nested["required"], json!(["optional"]));
        assert_eq!(
            nested["properties"]["optional"]["type"],
            json!(["string", "null"])
        );
        assert_eq!(nested["additionalProperties"], false);
    }
}
