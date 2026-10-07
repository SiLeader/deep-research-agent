use crate::ReActAgent;
use serde::de::DeserializeOwned;

impl ReActAgent {
    pub async fn get_output<T>(&self, message: String) -> anyhow::Result<T>
    where
        T: DeserializeOwned + 'static,
    {
        self.get_output_validated(message, |_: &T| Ok(())).await
    }

    /// Run until a submission deserializes as `T` and passes `validate`.
    pub async fn get_output_validated<T, V>(
        &self,
        message: String,
        validate: V,
    ) -> anyhow::Result<T>
    where
        T: DeserializeOwned + 'static,
        V: Fn(&T) -> anyhow::Result<()> + Send + Sync + 'static,
    {
        self.run_with_event_validated(message, validate, |_| async {})
            .await
    }
}
