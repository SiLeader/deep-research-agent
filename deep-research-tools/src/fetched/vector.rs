//! Keep EmveDb's non-Send storage on its owning thread. Only owned commands and
//! results cross threads; dropping the last sender releases the in-memory index.
use emvedb::{CreateOptions, EmveDb, SearchOptions};
use std::sync::mpsc;

type Job = Box<dyn FnOnce(&EmveDb) + Send>;

pub(super) struct VectorDb {
    jobs: mpsc::Sender<Job>,
    dimension: u32,
}

impl VectorDb {
    pub async fn create(dimension: u32) -> anyhow::Result<Self> {
        let (jobs, receiver) = mpsc::channel::<Job>();
        let (ready, initialized) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("fetched-vectors".into())
            .spawn(move || {
                let db = match EmveDb::create(
                    ":memory:",
                    &CreateOptions {
                        dimension,
                        ..Default::default()
                    },
                ) {
                    Ok(db) => db,
                    Err(error) => {
                        let _ = ready.send(Err(anyhow::anyhow!(error.to_string())));
                        return;
                    }
                };
                if ready.send(Ok(())).is_err() {
                    return;
                }
                for job in receiver {
                    job(&db);
                }
            })?;
        initialized.await??;
        Ok(Self { jobs, dimension })
    }

    async fn execute<T: Send + 'static>(
        &self,
        job: impl FnOnce(&EmveDb) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        let (reply, result) = tokio::sync::oneshot::channel();
        self.jobs
            .send(Box::new(move |db| {
                let _ = reply.send(job(db));
            }))
            .map_err(|_| anyhow::anyhow!("vector worker stopped"))?;
        result
            .await
            .map_err(|_| anyhow::anyhow!("vector worker stopped"))?
    }

    pub fn dimension(&self) -> u32 {
        self.dimension
    }

    pub async fn put(&self, id: u64, vector: &[f32]) -> anyhow::Result<()> {
        let vector = vector.to_vec();
        self.execute(move |db| Ok(db.put(id, &vector, &[])?)).await
    }

    // Drop cannot await. Queue rollback after preceding writes on the same
    // worker, without blocking the runtime or losing cleanup on cancellation.
    pub fn delete_on_drop(&self, id: u64) {
        let _ = self.jobs.send(Box::new(move |db| {
            let _ = db.delete(id);
        }));
    }

    pub async fn search(&self, vector: &[f32], limit: usize) -> anyhow::Result<Vec<u64>> {
        let vector = vector.to_vec();
        self.execute(move |db| {
            Ok(db
                .search(&vector, limit, &SearchOptions::default())?
                .into_iter()
                .map(|item| item.id)
                .collect())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, time::Duration};

    #[tokio::test(flavor = "current_thread")]
    async fn worker_wait_does_not_block_runtime() {
        let db = Arc::new(VectorDb::create(2).await.unwrap());
        let (started, running) = tokio::sync::oneshot::channel();
        let (release, wait) = mpsc::channel();
        let job = tokio::spawn({
            let db = db.clone();
            async move {
                db.execute(move |_| {
                    started.send(()).unwrap();
                    // A bounded wait also lets a blocking regression terminate.
                    let _ = wait.recv_timeout(Duration::from_secs(2));
                    Ok(())
                })
                .await
                .unwrap();
            }
        });
        running.await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(
            !job.is_finished(),
            "timer must run while vector worker is busy"
        );
        release.send(()).unwrap();
        job.await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_write_is_rolled_back_before_next_search() {
        let db = VectorDb::create(2).await.unwrap();
        // Hold the worker so cancellation happens with the put still queued.
        let (release, wait) = mpsc::channel();
        db.jobs
            .send(Box::new(move |_| {
                wait.recv().unwrap();
            }))
            .unwrap();
        let mut write = Box::pin(db.put(1, &[1.0, 0.0]));
        let pending = super::super::PendingVectors {
            db: Some(&db),
            ids: vec![1],
        };
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(std::future::Future::poll(write.as_mut(), &mut cx).is_pending());
        drop(write);
        drop(pending);
        release.send(()).unwrap();
        assert!(db.search(&[1.0, 0.0], 10).await.unwrap().is_empty());
    }
}
