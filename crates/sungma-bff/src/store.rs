use std::{fmt, sync::Arc};

use async_trait::async_trait;
use serde::Deserialize;
use sungma_clock::TimeSource;
use tower_sessions::{
    SessionStore,
    session::{Id, Record},
    session_store::Result,
};

use crate::{
    session::{SIGNED_IN, SignedIn},
    signed_out::SignedOut,
};

/// The session store the BFF saves through. A request saves the whole
/// session when it ends, as it read it, so this store drops a save for a
/// session that signed out while the request ran, and keeps the tokens of
/// a refresh saved since the request read the session.
#[derive(Clone)]
pub(crate) struct Guarded<S> {
    pub inner: S,
    pub signed_out: Arc<SignedOut>,
    pub time: Arc<dyn TimeSource>,
}

impl<S: fmt::Debug> fmt::Debug for Guarded<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Guarded")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl<S: SessionStore + Clone> SessionStore for Guarded<S> {
    async fn create(&self, record: &mut Record) -> Result<()> {
        self.inner.create(record).await
    }

    async fn save(&self, record: &Record) -> Result<()> {
        let id = record.id.to_string();
        if self.signed_out.contains(&id, self.time.wall()) {
            return Ok(());
        }
        match self.newer(record).await? {
            Some(record) => self.inner.save(&record).await,
            None => self.inner.save(record).await,
        }
    }

    async fn load(&self, id: &Id) -> Result<Option<Record>> {
        self.inner.load(id).await
    }

    async fn delete(&self, id: &Id) -> Result<()> {
        self.inner.delete(id).await
    }
}

impl<S: SessionStore> Guarded<S> {
    /// `record` with the stored tokens in place of its own, when a refresh
    /// stored newer ones after the request saving `record` read it.
    async fn newer(&self, record: &Record) -> Result<Option<Record>> {
        let Some(saving) = generation(record) else {
            return Ok(None);
        };
        let stored = self.inner.load(&record.id).await?;
        let Some(stored) =
            stored.filter(|stored| generation(stored).is_some_and(|stored| stored > saving))
        else {
            return Ok(None);
        };
        let mut record = record.clone();
        record
            .data
            .insert(SIGNED_IN.to_owned(), stored.data[SIGNED_IN].clone());
        Ok(Some(record))
    }
}

/// How many refreshes the tokens in `record` come after the sign-in.
fn generation(record: &Record) -> Option<u64> {
    let signed_in = SignedIn::deserialize(record.data.get(SIGNED_IN)?).ok()?;
    Some(signed_in.tokens.generation)
}

#[cfg(all(test, feature = "testkit"))]
mod tests {
    use std::time::Duration;

    use serde_json::json;
    use sungma_clock::ManualTime;
    use tower_sessions::{MemoryStore, cookie::time::OffsetDateTime};

    use super::*;

    fn guarded() -> Guarded<MemoryStore> {
        Guarded {
            inner: MemoryStore::default(),
            signed_out: Arc::new(SignedOut::new(Duration::from_secs(30))),
            time: Arc::new(ManualTime::new(0)),
        }
    }

    /// A session holding tokens with `access` that come `generation`
    /// refreshes after the sign-in.
    fn record(access: &str, generation: u64) -> Record {
        let signed_in = json!({
            "user": { "subject": "alice", "name": null, "username": null, "email": null },
            "tokens": {
                "access": access,
                "refresh": null,
                "refresh_at": null,
                "expires_at": null,
                "id_token": "",
                "generation": generation,
            },
        });
        Record {
            id: Id(1),
            data: [(SIGNED_IN.to_owned(), signed_in)].into(),
            expiry_date: OffsetDateTime::now_utc() + Duration::from_secs(60),
        }
    }

    async fn stored_access(store: &Guarded<MemoryStore>) -> String {
        let stored = store.load(&Id(1)).await.unwrap().unwrap();
        stored.data[SIGNED_IN]["tokens"]["access"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[tokio::test]
    async fn a_save_of_older_tokens_keeps_the_stored_ones() {
        let store = guarded();
        store.save(&record("new", 1)).await.unwrap();
        store.save(&record("old", 0)).await.unwrap();
        assert_eq!(stored_access(&store).await, "new");
    }

    #[tokio::test]
    async fn a_save_of_tokens_as_new_as_the_stored_ones_replaces_them() {
        let store = guarded();
        store.save(&record("first", 1)).await.unwrap();
        store.save(&record("second", 1)).await.unwrap();
        assert_eq!(stored_access(&store).await, "second");
        store.save(&record("third", 2)).await.unwrap();
        assert_eq!(stored_access(&store).await, "third");
    }

    #[tokio::test]
    async fn a_save_for_a_signed_out_session_is_dropped() {
        let store = guarded();
        store.signed_out.add(Id(1).to_string(), 0);
        store.save(&record("new", 0)).await.unwrap();
        assert!(store.load(&Id(1)).await.unwrap().is_none());
    }

    #[test]
    fn a_guarded_store_shows_the_store_it_guards() {
        assert!(format!("{:?}", guarded()).contains("MemoryStore"));
    }
}
