use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use tokio::sync::{mpsc::Sender, watch};

use crate::state::ClientToken;

pub(crate) type OutboundMessage = Arc<[u8]>;
pub(crate) type ClientWriter = Sender<OutboundMessage>;
pub(crate) type DeliveryRegistryHandle = Arc<Mutex<DeliveryRegistry>>;

pub(crate) struct DeliveryRegistry {
    pub(crate) by_addr: HashMap<SocketAddr, Vec<ClientDelivery>>,
}

pub(crate) struct ClientDelivery {
    pub(crate) addr: SocketAddr,
    pub(crate) token: ClientToken,
    pub(crate) writer: ClientWriter,
    pub(crate) disconnect: Option<watch::Sender<bool>>,
    pub(crate) ready: bool,
    pub(crate) pending_messages: Vec<OutboundMessage>,
}

pub(crate) fn new_registry() -> DeliveryRegistryHandle {
    Arc::new(Mutex::new(DeliveryRegistry {
        by_addr: HashMap::new(),
    }))
}
