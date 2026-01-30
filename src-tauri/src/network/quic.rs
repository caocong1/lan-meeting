//! QUIC-based P2P transport
//! Low-latency, encrypted communication using quinn

use super::NetworkError;
use parking_lot::RwLock;
use quinn::{ClientConfig, Connection, Endpoint, RecvStream, SendStream, ServerConfig};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

/// Default QUIC port
pub const DEFAULT_PORT: u16 = 19876;

/// QUIC connection configuration
#[derive(Debug, Clone)]
pub struct QuicConfig {
    pub bind_addr: SocketAddr,
    pub max_idle_timeout: Duration,
    pub keep_alive_interval: Duration,
}

impl Default for QuicConfig {
    fn default() -> Self {
        Self {
            bind_addr: format!("0.0.0.0:{}", DEFAULT_PORT).parse().unwrap(),
            max_idle_timeout: Duration::from_secs(30),
            keep_alive_interval: Duration::from_secs(5),
        }
    }
}

/// Connection state
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnectionState {
    Connecting,
    Connected,
    Disconnected,
}

/// Active connections registry
pub static CONNECTIONS: once_cell::sync::Lazy<Arc<RwLock<HashMap<String, Arc<QuicConnection>>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

/// QUIC endpoint for P2P connections
pub struct QuicEndpoint {
    endpoint: Endpoint,
    config: QuicConfig,
}

impl QuicEndpoint {
    /// Create a new QUIC endpoint (both server and client)
    pub async fn new(config: QuicConfig) -> Result<Self, NetworkError> {
        // Generate self-signed certificate
        let (server_config, _cert) = Self::generate_server_config()?;

        // Create endpoint with server config
        let endpoint = Endpoint::server(server_config, config.bind_addr)
            .map_err(|e| NetworkError::ConnectionFailed(format!("Failed to create endpoint: {}", e)))?;

        log::info!("QUIC endpoint created on {}", config.bind_addr);

        Ok(Self { endpoint, config })
    }

    /// Generate server configuration with self-signed certificate
    fn generate_server_config() -> Result<(ServerConfig, CertificateDer<'static>), NetworkError> {
        // Generate self-signed certificate
        let cert = rcgen::generate_simple_self_signed(vec!["lan-meeting".to_string()])
            .map_err(|e| NetworkError::ConnectionFailed(format!("Failed to generate cert: {}", e)))?;

        let cert_der = CertificateDer::from(cert.cert);
        let key_der = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());

        // Create rustls server config
        let mut server_crypto = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der.clone()], key_der.into())
            .map_err(|e| NetworkError::ConnectionFailed(format!("TLS config error: {}", e)))?;

        server_crypto.alpn_protocols = vec![b"lan-meeting".to_vec()];

        // Create quinn server config with transport settings
        let mut server_config = ServerConfig::with_crypto(Arc::new(
            quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto)
                .map_err(|e| NetworkError::ConnectionFailed(format!("QUIC config error: {}", e)))?,
        ));

        // Configure transport for low latency video streaming
        let transport = Self::create_transport_config();
        server_config.transport_config(Arc::new(transport));

        Ok((server_config, cert_der))
    }

    /// Create shared transport configuration for both server and client
    fn create_transport_config() -> quinn::TransportConfig {
        let mut transport = quinn::TransportConfig::default();
        transport.max_idle_timeout(Some(Duration::from_secs(30).try_into().unwrap()));
        transport.keep_alive_interval(Some(Duration::from_secs(5)));
        // Allow more concurrent streams for video frame broadcasting
        transport.max_concurrent_bidi_streams(1024u32.into());
        transport.max_concurrent_uni_streams(1024u32.into());
        // Enable datagrams for future low-latency frame delivery
        transport.datagram_receive_buffer_size(Some(65536));
        transport
    }

    /// Create client configuration (accepts any certificate for LAN use)
    fn create_client_config() -> Result<ClientConfig, NetworkError> {
        // For LAN use, we skip certificate verification
        // In production, you'd want proper certificate validation
        let mut crypto = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
            .with_no_client_auth();

        // IMPORTANT: Must match server's ALPN protocols
        crypto.alpn_protocols = vec![b"lan-meeting".to_vec()];

        let mut client_config = ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(crypto)
                .map_err(|e| NetworkError::ConnectionFailed(format!("Client config error: {}", e)))?,
        ));

        // Configure transport for low latency video streaming
        let transport = Self::create_transport_config();
        client_config.transport_config(Arc::new(transport));

        Ok(client_config)
    }

    /// Connect to a remote peer
    pub async fn connect(&self, addr: SocketAddr) -> Result<Arc<QuicConnection>, NetworkError> {
        log::info!("Connecting to {}", addr);

        let client_config = Self::create_client_config()?;

        let connection = self
            .endpoint
            .connect_with(client_config, addr, "lan-meeting")
            .map_err(|e| NetworkError::ConnectionFailed(format!("Connect error: {}", e)))?
            .await
            .map_err(|e| NetworkError::ConnectionFailed(format!("Connection failed: {}", e)))?;

        let remote_addr = connection.remote_address();
        log::info!("Connected to {}", remote_addr);

        let conn = Arc::new(QuicConnection::new(connection));

        // Store connection
        let conn_id = remote_addr.to_string();
        CONNECTIONS.write().insert(conn_id, conn.clone());

        Ok(conn)
    }

    /// Accept incoming connections
    pub async fn accept(&self) -> Result<Arc<QuicConnection>, NetworkError> {
        let incoming = self
            .endpoint
            .accept()
            .await
            .ok_or_else(|| NetworkError::ConnectionFailed("Endpoint closed".to_string()))?;

        let connection = incoming
            .await
            .map_err(|e| NetworkError::ConnectionFailed(format!("Accept failed: {}", e)))?;

        let remote_addr = connection.remote_address();
        log::info!("Accepted connection from {}", remote_addr);

        let conn = Arc::new(QuicConnection::new(connection));

        // Store connection
        let conn_id = remote_addr.to_string();
        CONNECTIONS.write().insert(conn_id, conn.clone());

        Ok(conn)
    }

    /// Start accepting connections in background
    pub fn start_server(
        self: Arc<Self>,
        on_connection: impl Fn(Arc<QuicConnection>) + Send + Sync + 'static,
    ) {
        let on_connection = Arc::new(on_connection);

        tokio::spawn(async move {
            loop {
                // Wait for an incoming connection attempt
                let incoming = match self.endpoint.accept().await {
                    Some(incoming) => incoming,
                    None => {
                        log::info!("QUIC endpoint closed, stopping server");
                        break;
                    }
                };

                // Complete the connection handshake (may fail for individual connections)
                match incoming.await {
                    Ok(connection) => {
                        let remote_addr = connection.remote_address();
                        log::info!("Accepted connection from {}", remote_addr);

                        let conn = Arc::new(QuicConnection::new(connection));
                        let conn_id = remote_addr.to_string();
                        CONNECTIONS.write().insert(conn_id, conn.clone());

                        let on_connection = on_connection.clone();
                        tokio::spawn(async move {
                            on_connection(conn);
                        });
                    }
                    Err(e) => {
                        // Individual connection failed - continue accepting others
                        log::warn!("Failed to accept individual connection: {}", e);
                        continue;
                    }
                }
            }
        });
    }

    /// Get endpoint local address
    pub fn local_addr(&self) -> SocketAddr {
        self.endpoint.local_addr().unwrap_or(self.config.bind_addr)
    }

    /// Close the endpoint
    pub fn close(&self) {
        self.endpoint.close(0u32.into(), b"shutdown");
    }
}

/// Active QUIC connection to a peer
pub struct QuicConnection {
    connection: Connection,
    state: RwLock<ConnectionState>,
}

impl QuicConnection {
    fn new(connection: Connection) -> Self {
        Self {
            connection,
            state: RwLock::new(ConnectionState::Connected),
        }
    }

    /// Get connection state
    pub fn state(&self) -> ConnectionState {
        *self.state.read()
    }

    /// Get remote address
    pub fn remote_addr(&self) -> SocketAddr {
        self.connection.remote_address()
    }

    /// Open a new bidirectional stream
    pub async fn open_bi_stream(&self) -> Result<QuicStream, NetworkError> {
        let (send, recv) = self
            .connection
            .open_bi()
            .await
            .map_err(|e| NetworkError::ConnectionFailed(format!("Failed to open stream: {}", e)))?;

        Ok(QuicStream::new(send, recv))
    }

    /// Accept an incoming bidirectional stream
    pub async fn accept_bi_stream(&self) -> Result<QuicStream, NetworkError> {
        let (send, recv) = self
            .connection
            .accept_bi()
            .await
            .map_err(|e| NetworkError::ConnectionFailed(format!("Failed to accept stream: {}", e)))?;

        Ok(QuicStream::new(send, recv))
    }

    /// Open a unidirectional send stream
    pub async fn open_uni_stream(&self) -> Result<SendStream, NetworkError> {
        self.connection
            .open_uni()
            .await
            .map_err(|e| NetworkError::ConnectionFailed(format!("Failed to open uni stream: {}", e)))
    }

    /// Accept a unidirectional receive stream
    pub async fn accept_uni_stream(&self) -> Result<RecvStream, NetworkError> {
        self.connection
            .accept_uni()
            .await
            .map_err(|e| NetworkError::ConnectionFailed(format!("Failed to accept uni stream: {}", e)))
    }

    /// Send datagram (unreliable, for video frames)
    pub fn send_datagram(&self, data: bytes::Bytes) -> Result<(), NetworkError> {
        self.connection
            .send_datagram(data)
            .map_err(|e| NetworkError::ConnectionFailed(format!("Failed to send datagram: {}", e)))
    }

    /// Receive datagram
    pub async fn recv_datagram(&self) -> Result<bytes::Bytes, NetworkError> {
        self.connection
            .read_datagram()
            .await
            .map_err(|e| NetworkError::ConnectionFailed(format!("Failed to recv datagram: {}", e)))
    }

    /// Close the connection
    pub fn close(&self) {
        *self.state.write() = ConnectionState::Disconnected;
        self.connection.close(0u32.into(), b"done");
    }

    /// Check if the underlying QUIC connection is still alive
    /// Unlike is_connected() which only tracks explicit close(), this checks
    /// the actual connection state including natural timeouts
    pub fn is_alive(&self) -> bool {
        self.connection.close_reason().is_none()
    }

    /// Check if connection is alive (legacy - prefer is_alive())
    pub fn is_connected(&self) -> bool {
        self.is_alive()
    }
}

/// QUIC bidirectional stream for data transmission
pub struct QuicStream {
    send: SendStream,
    recv: RecvStream,
}

impl QuicStream {
    fn new(send: SendStream, recv: RecvStream) -> Self {
        Self { send, recv }
    }

    /// Send data on this stream
    pub async fn send(&mut self, data: &[u8]) -> Result<(), NetworkError> {
        self.send
            .write_all(data)
            .await
            .map_err(|e| NetworkError::ConnectionFailed(format!("Send error: {}", e)))
    }

    /// Send data with length prefix (for framed messages)
    pub async fn send_framed(&mut self, data: &[u8]) -> Result<(), NetworkError> {
        let len = data.len() as u32;
        self.send
            .write_all(&len.to_be_bytes())
            .await
            .map_err(|e| NetworkError::ConnectionFailed(format!("Send length error: {}", e)))?;
        self.send
            .write_all(data)
            .await
            .map_err(|e| NetworkError::ConnectionFailed(format!("Send data error: {}", e)))
    }

    /// Receive data from this stream
    pub async fn recv(&mut self, buf: &mut [u8]) -> Result<usize, NetworkError> {
        self.recv
            .read(buf)
            .await
            .map_err(|e| NetworkError::ConnectionFailed(format!("Recv error: {}", e)))?
            .ok_or_else(|| NetworkError::ConnectionFailed("Stream closed".to_string()))
    }

    /// Receive framed message (with length prefix)
    pub async fn recv_framed(&mut self) -> Result<Vec<u8>, NetworkError> {
        let mut len_buf = [0u8; 4];
        self.recv
            .read_exact(&mut len_buf)
            .await
            .map_err(|e| NetworkError::ConnectionFailed(format!("Recv length error: {}", e)))?;

        let len = u32::from_be_bytes(len_buf) as usize;
        let mut data = vec![0u8; len];
        self.recv
            .read_exact(&mut data)
            .await
            .map_err(|e| NetworkError::ConnectionFailed(format!("Recv data error: {}", e)))?;

        Ok(data)
    }

    /// Finish sending (close send side)
    pub async fn finish(&mut self) -> Result<(), NetworkError> {
        self.send
            .finish()
            .map_err(|e| NetworkError::ConnectionFailed(format!("Finish error: {}", e)))
    }
}

/// Get connection by ID
pub fn get_connection(id: &str) -> Option<Arc<QuicConnection>> {
    CONNECTIONS.read().get(id).cloned()
}

/// Remove connection
pub fn remove_connection(id: &str) {
    CONNECTIONS.write().remove(id);
}

/// Get all active connections
pub fn get_all_connections() -> Vec<Arc<QuicConnection>> {
    CONNECTIONS.read().values().cloned().collect()
}

/// Broadcast a message to all connected peers
pub async fn broadcast_message(data: &[u8]) -> Vec<Result<(), super::NetworkError>> {
    // Remove dead connections first
    cleanup_dead_connections();

    let connections = get_all_connections();
    let mut results = Vec::with_capacity(connections.len());

    for conn in connections {
        let result = async {
            let mut stream = conn.open_bi_stream().await?;
            stream.send_framed(data).await?;
            stream.finish().await?;
            Ok(())
        }
        .await;
        results.push(result);
    }

    results
}

/// Send a message to a specific peer by connection ID or IP address
/// Accepts either "ip:port" or just "ip" - if only IP is provided, searches for matching connection
pub async fn send_to_peer(peer_id: &str, data: &[u8]) -> Result<(), super::NetworkError> {
    let conn = find_connection(peer_id).ok_or_else(|| {
        super::NetworkError::ConnectionFailed(format!("Peer not found: {}", peer_id))
    })?;

    // Check if the connection is still alive
    if !conn.is_alive() {
        log::warn!("Connection to {} is dead, removing", peer_id);
        remove_connection_by_ip(peer_id);
        return Err(super::NetworkError::ConnectionFailed(format!(
            "Connection to {} has timed out",
            peer_id
        )));
    }

    // Use a timeout for stream opening to fail fast instead of waiting
    // for the full connection idle timeout (30s)
    let mut stream = tokio::time::timeout(
        Duration::from_secs(3),
        conn.open_bi_stream(),
    )
    .await
    .map_err(|_| {
        log::warn!("Timeout opening stream to {}, connection may be dead", peer_id);
        super::NetworkError::ConnectionFailed(format!(
            "Stream open to {} timed out - peer may be unreachable (check firewall settings)",
            peer_id
        ))
    })??;

    stream.send_framed(data).await?;
    stream.finish().await?;
    Ok(())
}

/// Find a connection by ID (exact match) or by IP prefix
pub fn find_connection(peer_id: &str) -> Option<Arc<QuicConnection>> {
    // Try exact match first (ip:port format)
    if let Some(conn) = get_connection(peer_id) {
        return Some(conn);
    }
    // If no exact match, try to find by IP prefix (when only IP is provided without port)
    let connections = CONNECTIONS.read();
    connections
        .iter()
        .find(|(key, _)| key.starts_with(&format!("{}:", peer_id)))
        .map(|(_, conn)| conn.clone())
}

/// Remove dead connections from the registry
pub fn cleanup_dead_connections() {
    let dead_keys: Vec<String> = {
        let connections = CONNECTIONS.read();
        connections
            .iter()
            .filter(|(_, conn)| !conn.is_alive())
            .map(|(key, _)| key.clone())
            .collect()
    };

    if !dead_keys.is_empty() {
        let mut connections = CONNECTIONS.write();
        for key in &dead_keys {
            log::info!("Removing dead connection: {}", key);
            connections.remove(key);
        }
    }
}

/// Remove connection by IP address (matches ip:port keys)
pub fn remove_connection_by_ip(ip: &str) {
    let mut connections = CONNECTIONS.write();
    connections.retain(|key, _| !key.starts_with(&format!("{}:", ip)) && key != ip);
}

/// Skip server certificate verification for LAN use
#[derive(Debug)]
struct SkipServerVerification;

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
        ]
    }
}
