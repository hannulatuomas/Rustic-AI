pub mod types;

use tokio::sync::broadcast;

use crate::events::types::Event;

#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<Event>,
}

impl EventBus {
    pub fn new(buffer_size: usize) -> Self {
        let (sender, _) = broadcast::channel(buffer_size.max(1));
        Self { sender }
    }

    pub fn publish(&self, event: Event) {
        let _ = self.sender.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(128)
    }
}
