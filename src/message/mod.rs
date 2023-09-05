pub mod model;

use std::sync::Arc;

use dashmap::DashMap;
use model::client::Message;
use tokio::sync::mpsc::{self, error::SendError};
use uuid::Uuid;

lazy_static::lazy_static! {
static ref _RAW_MAILBOX: Arc<MailBoxRaw> = {
    tracing::trace!("initializing raw mailbox resources...");
    Arc::new(if cfg!(debug_assertions) { MailBoxRaw::new() } else {
        tracing::debug!("pre-allocating memory to prevent repeated allocations (prevent global locking of dashmap)");
        MailBoxRaw::with_capacity(1024)
    })
};
}

pub trait ReaderType {}

pub struct Receiver;
pub struct Sender;

impl ReaderType for Receiver {}
impl ReaderType for Sender {}

struct MailBoxRaw {
    rx_map: DashMap<Uuid, mpsc::UnboundedReceiver<Message>>,
    tx_map: DashMap<Uuid, mpsc::UnboundedSender<Message>>,
}

#[rustfmt::skip]
#[derive(Debug)]
#[derive(thiserror::Error)]
pub enum Error {
    #[error("client {0} does not exist")]
    DoesNotExist(Uuid),

    #[error("failed sending message: {0:?}")]
    SenderError(#[from] SendError<Message>),
}

#[derive(Clone)]
pub struct MailBox<T: ReaderType> {
    _inner: Arc<MailBoxRaw>,
    _type: std::marker::PhantomData<T>,
}

impl MailBoxRaw {
    fn new() -> Self {
        Self {
            rx_map: DashMap::new(),
            tx_map: DashMap::new(),
        }
    }

    fn with_capacity(capacity: usize) -> Self {
        Self {
            rx_map: DashMap::with_capacity(capacity),
            tx_map: DashMap::with_capacity(capacity),
        }
    }
}

impl<T: ReaderType> MailBox<T> {
    pub fn instance() -> Self {
        let _inner = Arc::clone(&_RAW_MAILBOX);

        Self {
            _inner,
            _type: Default::default(),
        }
    }
}

impl MailBox<Receiver> {
    pub async fn recv(&self, client_id: Uuid) -> Option<Message> {
        self._inner.rx_map.get_mut(&client_id)?.recv().await
    }
}

impl MailBox<Sender> {
    pub async fn send(&self, client_id: Uuid, message: Message) -> Result<(), Error> {
        self._inner
            .tx_map
            .get_mut(&client_id)
            .ok_or(Error::DoesNotExist(client_id))?
            .send(message)?;

        Ok(())
    }

    pub fn add_client(&self, client_id: Uuid) {
        let (tx, rx) = mpsc::unbounded_channel();

        self._inner.rx_map.insert(client_id, rx);
        self._inner.tx_map.insert(client_id, tx);
    }

    #[tracing::instrument(skip(self))]
    pub async fn remove_client(&self, client_id: Uuid) {
        if let Some((id, mut rx)) = self._inner.rx_map.remove(&client_id) {
            tracing::debug!("closing mailbox receiver...");
            rx.close();
        } else {
            tracing::warn!("receiver does not exist");
        }

        if let Some((id, tx)) = self._inner.tx_map.remove(&client_id) {
            tracing::debug!("closing mailbox sender...");
            tx.closed().await;
        }
    }
}