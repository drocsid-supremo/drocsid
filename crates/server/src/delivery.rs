use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex, mpsc::SyncSender},
};

use crate::state::ClientToken;

pub(crate) type ClientWriter = SyncSender<Vec<u8>>;
pub(crate) type DeliveryRegistryHandle = Arc<Mutex<DeliveryRegistry>>;

pub(crate) struct DeliveryRegistry {
    pub(crate) by_addr: HashMap<SocketAddr, Vec<ClientDelivery>>,
}

pub(crate) struct ClientDelivery {
    pub(crate) addr: SocketAddr,
    pub(crate) token: ClientToken,
    pub(crate) writer: ClientWriter,
    pub(crate) ready: bool,
    pub(crate) pending_messages: Vec<Vec<u8>>,
}

pub(crate) fn new_registry() -> DeliveryRegistryHandle {
    Arc::new(Mutex::new(DeliveryRegistry {
        by_addr: HashMap::new(),
    }))
}
