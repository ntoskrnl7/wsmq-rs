use std::net::SocketAddr;

#[macro_export]
macro_rules! define_method {
    ($prop:ident, $type:ty) => {
        pub fn $prop(mut self, callback: $type) -> Self {
            self.$prop = Some(callback);
            self
        }
    };
}

pub type OnStarted = Box<dyn Fn() + Send + Sync>;
pub type OnConnect = Box<dyn Fn(SocketAddr) + Send + Sync>;
pub type OnDisconnect = Box<dyn Fn(SocketAddr) + Send + Sync>;
pub type OnError = Box<dyn Fn(Box<dyn std::error::Error>) + Send + Sync>;
