// Network module
// QUIC-based P2P communication with mDNS discovery

pub mod discovery;
pub mod protocol;
pub mod quic;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum NetworkError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Discovery error: {0}")]
    DiscoveryError(String),
    #[error("Protocol error: {0}")]
    ProtocolError(String),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}
