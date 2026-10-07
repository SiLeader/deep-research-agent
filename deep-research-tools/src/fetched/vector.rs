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
    pub fn create(dimension: u32) -> anyhow::Result<Self> {
        let (jobs, receiver) = mpsc::channel::<Job>();
        let (ready, initialized) = mpsc::sync_channel(1);
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
        initialized.recv()??;
        Ok(Self { jobs, dimension })
    }

    fn execute<T: Send + 'static>(
        &self,
        job: impl FnOnce(&EmveDb) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        let (reply, result) = mpsc::sync_channel(1);
        self.jobs
            .send(Box::new(move |db| {
                let _ = reply.send(job(db));
            }))
            .map_err(|_| anyhow::anyhow!("vector worker stopped"))?;
        result
            .recv()
            .map_err(|_| anyhow::anyhow!("vector worker stopped"))?
    }

    pub fn dimension(&self) -> u32 {
        self.dimension
    }

    pub fn put(&self, id: u64, vector: &[f32]) -> anyhow::Result<()> {
        let vector = vector.to_vec();
        self.execute(move |db| Ok(db.put(id, &vector, &[])?))
    }

    pub fn delete(&self, id: u64) -> anyhow::Result<()> {
        self.execute(move |db| {
            db.delete(id)?;
            Ok(())
        })
    }

    pub fn search(&self, vector: &[f32], limit: usize) -> anyhow::Result<Vec<u64>> {
        let vector = vector.to_vec();
        self.execute(move |db| {
            Ok(db
                .search(&vector, limit, &SearchOptions::default())?
                .into_iter()
                .map(|item| item.id)
                .collect())
        })
    }
}
