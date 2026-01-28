// File transfer module
// P2P file sharing with resume support

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

/// Chunk size for file transfer (64KB)
pub const CHUNK_SIZE: usize = 64 * 1024;

/// Maximum concurrent transfers
pub const MAX_CONCURRENT_TRANSFERS: usize = 5;

#[derive(Error, Debug)]
pub enum TransferError {
    #[error("Transfer failed: {0}")]
    TransferFailed(String),
    #[error("File not found: {0}")]
    FileNotFound(String),
    #[error("Transfer not found: {0}")]
    TransferNotFound(String),
    #[error("Transfer cancelled")]
    Cancelled,
    #[error("Checksum mismatch")]
    ChecksumMismatch,
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// File information for transfer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    /// Unique file transfer ID
    pub id: String,
    /// Original file name
    pub name: String,
    /// File size in bytes
    pub size: u64,
    /// SHA-256 checksum
    pub checksum: String,
    /// MIME type (optional)
    pub mime_type: Option<String>,
}

impl FileInfo {
    /// Create FileInfo from a file path
    pub fn from_path(path: &Path) -> Result<Self, TransferError> {
        let file = File::open(path).map_err(|_| {
            TransferError::FileNotFound(path.display().to_string())
        })?;

        let metadata = file.metadata()?;
        let size = metadata.len();

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        // Calculate checksum
        let checksum = calculate_file_checksum(path)?;

        // Guess MIME type
        let mime_type = mime_guess::from_path(path)
            .first()
            .map(|m| m.to_string());

        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            size,
            checksum,
            mime_type,
        })
    }
}

/// Calculate SHA-256 checksum of a file
fn calculate_file_checksum(path: &Path) -> Result<String, TransferError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

/// Transfer status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferStatus {
    /// Waiting to start
    Pending,
    /// Waiting for peer to accept
    Offered,
    /// Transfer in progress
    InProgress,
    /// Transfer completed successfully
    Completed,
    /// Transfer failed
    Failed,
    /// Transfer cancelled by user
    Cancelled,
}

/// Transfer direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferDirection {
    /// Sending file to peer
    Outgoing,
    /// Receiving file from peer
    Incoming,
}

/// File transfer state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTransfer {
    /// File information
    pub info: FileInfo,
    /// Transfer status
    pub status: TransferStatus,
    /// Transfer direction
    pub direction: TransferDirection,
    /// Progress (0.0 - 1.0)
    pub progress: f32,
    /// Bytes transferred
    pub bytes_transferred: u64,
    /// Peer device ID
    pub peer_id: String,
    /// Local file path (for sending) or destination path (for receiving)
    pub local_path: Option<String>,
    /// Error message if failed
    pub error: Option<String>,
}

impl FileTransfer {
    /// Create a new outgoing transfer
    pub fn new_outgoing(info: FileInfo, peer_id: &str, local_path: &str) -> Self {
        Self {
            info,
            status: TransferStatus::Pending,
            direction: TransferDirection::Outgoing,
            progress: 0.0,
            bytes_transferred: 0,
            peer_id: peer_id.to_string(),
            local_path: Some(local_path.to_string()),
            error: None,
        }
    }

    /// Create a new incoming transfer
    pub fn new_incoming(info: FileInfo, peer_id: &str) -> Self {
        Self {
            info,
            status: TransferStatus::Offered,
            direction: TransferDirection::Incoming,
            progress: 0.0,
            bytes_transferred: 0,
            peer_id: peer_id.to_string(),
            local_path: None,
            error: None,
        }
    }

    /// Update progress
    pub fn update_progress(&mut self, bytes: u64) {
        self.bytes_transferred = bytes;
        if self.info.size > 0 {
            self.progress = bytes as f32 / self.info.size as f32;
        } else {
            self.progress = 1.0;
        }
    }

    /// Mark as in progress
    pub fn start(&mut self) {
        self.status = TransferStatus::InProgress;
    }

    /// Mark as completed
    pub fn complete(&mut self) {
        self.status = TransferStatus::Completed;
        self.progress = 1.0;
        self.bytes_transferred = self.info.size;
    }

    /// Mark as failed
    pub fn fail(&mut self, error: &str) {
        self.status = TransferStatus::Failed;
        self.error = Some(error.to_string());
    }

    /// Mark as cancelled
    pub fn cancel(&mut self) {
        self.status = TransferStatus::Cancelled;
    }
}

/// File sender for reading and sending file chunks
pub struct FileSender {
    file: File,
    info: FileInfo,
    #[allow(dead_code)]
    path: PathBuf,
}

impl FileSender {
    /// Create a new file sender
    pub fn new(path: &Path) -> Result<Self, TransferError> {
        let info = FileInfo::from_path(path)?;
        let file = File::open(path)?;

        Ok(Self {
            file,
            info,
            path: path.to_path_buf(),
        })
    }

    /// Get file info
    pub fn info(&self) -> &FileInfo {
        &self.info
    }

    /// Get a chunk at the specified offset
    pub fn get_chunk(&mut self, offset: u64) -> Result<Vec<u8>, TransferError> {
        self.file.seek(SeekFrom::Start(offset))?;

        let remaining = self.info.size.saturating_sub(offset) as usize;
        let chunk_size = remaining.min(CHUNK_SIZE);

        let mut buffer = vec![0u8; chunk_size];
        let bytes_read = self.file.read(&mut buffer)?;
        buffer.truncate(bytes_read);

        Ok(buffer)
    }

    /// Get total number of chunks
    pub fn chunk_count(&self) -> u64 {
        (self.info.size + CHUNK_SIZE as u64 - 1) / CHUNK_SIZE as u64
    }
}

/// File receiver for writing received chunks
pub struct FileReceiver {
    file: File,
    info: FileInfo,
    path: PathBuf,
    bytes_received: u64,
    received_chunks: Vec<bool>,
}

impl FileReceiver {
    /// Create a new file receiver
    pub fn new(info: FileInfo, dest_path: &Path) -> Result<Self, TransferError> {
        // Create parent directories if needed
        if let Some(parent) = dest_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Create/truncate the destination file
        let file = File::create(dest_path)?;

        // Pre-allocate file size
        file.set_len(info.size)?;

        let chunk_count = ((info.size + CHUNK_SIZE as u64 - 1) / CHUNK_SIZE as u64) as usize;

        Ok(Self {
            file,
            info,
            path: dest_path.to_path_buf(),
            bytes_received: 0,
            received_chunks: vec![false; chunk_count],
        })
    }

    /// Write a chunk at the specified offset
    pub fn write_chunk(&mut self, offset: u64, data: &[u8]) -> Result<(), TransferError> {
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(data)?;

        // Mark chunk as received
        let chunk_index = (offset / CHUNK_SIZE as u64) as usize;
        if chunk_index < self.received_chunks.len() {
            self.received_chunks[chunk_index] = true;
        }

        // Update bytes received
        self.bytes_received += data.len() as u64;

        Ok(())
    }

    /// Get bytes received
    pub fn bytes_received(&self) -> u64 {
        self.bytes_received
    }

    /// Check if all chunks are received
    pub fn is_complete(&self) -> bool {
        self.received_chunks.iter().all(|&received| received)
    }

    /// Get missing chunk offsets (for resume)
    pub fn missing_chunks(&self) -> Vec<u64> {
        self.received_chunks
            .iter()
            .enumerate()
            .filter(|(_, received)| !**received)
            .map(|(i, _)| i as u64 * CHUNK_SIZE as u64)
            .collect()
    }

    /// Verify the received file checksum
    pub fn verify(&mut self) -> Result<bool, TransferError> {
        // Flush and sync file
        self.file.sync_all()?;

        // Calculate checksum
        let checksum = calculate_file_checksum(&self.path)?;
        Ok(checksum == self.info.checksum)
    }

    /// Finalize the transfer
    pub fn finalize(&mut self) -> Result<(), TransferError> {
        self.file.sync_all()?;

        if !self.verify()? {
            return Err(TransferError::ChecksumMismatch);
        }

        Ok(())
    }
}

/// Transfer manager for handling multiple concurrent transfers
pub struct TransferManager {
    /// Active transfers (file_id -> transfer)
    transfers: RwLock<HashMap<String, FileTransfer>>,
    /// Active senders (file_id -> sender)
    senders: RwLock<HashMap<String, FileSender>>,
    /// Active receivers (file_id -> receiver)
    receivers: RwLock<HashMap<String, FileReceiver>>,
    /// Default download directory
    download_dir: PathBuf,
}

impl TransferManager {
    /// Create a new transfer manager
    pub fn new() -> Self {
        let download_dir = dirs::download_dir()
            .unwrap_or_else(|| PathBuf::from("."));

        Self {
            transfers: RwLock::new(HashMap::new()),
            senders: RwLock::new(HashMap::new()),
            receivers: RwLock::new(HashMap::new()),
            download_dir,
        }
    }

    /// Set download directory
    pub fn set_download_dir(&mut self, path: PathBuf) {
        self.download_dir = path;
    }

    /// Get download directory
    pub fn download_dir(&self) -> &Path {
        &self.download_dir
    }

    /// Offer a file for transfer (outgoing)
    pub fn offer_file(&self, path: &Path, peer_id: &str) -> Result<FileTransfer, TransferError> {
        // Create sender
        let sender = FileSender::new(path)?;
        let info = sender.info().clone();
        let file_id = info.id.clone();

        // Create transfer record
        let transfer = FileTransfer::new_outgoing(
            info,
            peer_id,
            &path.to_string_lossy(),
        );

        // Store
        self.transfers.write().insert(file_id.clone(), transfer.clone());
        self.senders.write().insert(file_id, sender);

        Ok(transfer)
    }

    /// Receive a file offer (incoming)
    pub fn receive_offer(&self, info: FileInfo, peer_id: &str) -> FileTransfer {
        let file_id = info.id.clone();

        let transfer = FileTransfer::new_incoming(info, peer_id);
        self.transfers.write().insert(file_id, transfer.clone());

        transfer
    }

    /// Accept an incoming file transfer
    pub fn accept_transfer(&self, file_id: &str, dest_path: Option<&Path>) -> Result<(), TransferError> {
        let mut transfers = self.transfers.write();
        let transfer = transfers
            .get_mut(file_id)
            .ok_or_else(|| TransferError::TransferNotFound(file_id.to_string()))?;

        if transfer.direction != TransferDirection::Incoming {
            return Err(TransferError::TransferFailed(
                "Cannot accept outgoing transfer".to_string(),
            ));
        }

        // Determine destination path
        let dest = dest_path
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| self.download_dir.join(&transfer.info.name));

        // Create receiver
        let receiver = FileReceiver::new(transfer.info.clone(), &dest)?;

        transfer.local_path = Some(dest.to_string_lossy().to_string());
        transfer.start();

        self.receivers.write().insert(file_id.to_string(), receiver);

        Ok(())
    }

    /// Reject an incoming file transfer
    pub fn reject_transfer(&self, file_id: &str) -> Result<(), TransferError> {
        let mut transfers = self.transfers.write();
        let transfer = transfers
            .get_mut(file_id)
            .ok_or_else(|| TransferError::TransferNotFound(file_id.to_string()))?;

        transfer.cancel();
        Ok(())
    }

    /// Get a chunk for sending
    pub fn get_chunk(&self, file_id: &str, offset: u64) -> Result<Vec<u8>, TransferError> {
        let mut senders = self.senders.write();
        let sender = senders
            .get_mut(file_id)
            .ok_or_else(|| TransferError::TransferNotFound(file_id.to_string()))?;

        sender.get_chunk(offset)
    }

    /// Write a received chunk
    pub fn write_chunk(&self, file_id: &str, offset: u64, data: &[u8]) -> Result<u64, TransferError> {
        let mut receivers = self.receivers.write();
        let receiver = receivers
            .get_mut(file_id)
            .ok_or_else(|| TransferError::TransferNotFound(file_id.to_string()))?;

        receiver.write_chunk(offset, data)?;
        let bytes = receiver.bytes_received();

        // Update transfer progress
        drop(receivers);
        let mut transfers = self.transfers.write();
        if let Some(transfer) = transfers.get_mut(file_id) {
            transfer.update_progress(bytes);
        }

        Ok(bytes)
    }

    /// Complete a transfer
    pub fn complete_transfer(&self, file_id: &str) -> Result<(), TransferError> {
        // Finalize receiver if incoming
        {
            let mut receivers = self.receivers.write();
            if let Some(receiver) = receivers.get_mut(file_id) {
                receiver.finalize()?;
            }
        }

        // Update transfer status
        let mut transfers = self.transfers.write();
        if let Some(transfer) = transfers.get_mut(file_id) {
            transfer.complete();
        }

        // Clean up sender/receiver
        self.senders.write().remove(file_id);
        self.receivers.write().remove(file_id);

        Ok(())
    }

    /// Cancel a transfer
    pub fn cancel_transfer(&self, file_id: &str) -> Result<(), TransferError> {
        let mut transfers = self.transfers.write();
        if let Some(transfer) = transfers.get_mut(file_id) {
            transfer.cancel();
        }

        // Clean up
        self.senders.write().remove(file_id);
        self.receivers.write().remove(file_id);

        Ok(())
    }

    /// Get transfer by ID
    pub fn get_transfer(&self, file_id: &str) -> Option<FileTransfer> {
        self.transfers.read().get(file_id).cloned()
    }

    /// Get all transfers
    pub fn get_all_transfers(&self) -> Vec<FileTransfer> {
        self.transfers.read().values().cloned().collect()
    }

    /// Get active transfers
    pub fn get_active_transfers(&self) -> Vec<FileTransfer> {
        self.transfers
            .read()
            .values()
            .filter(|t| matches!(t.status, TransferStatus::InProgress | TransferStatus::Offered))
            .cloned()
            .collect()
    }

    /// Remove completed/cancelled/failed transfers
    pub fn cleanup_finished(&self) {
        let mut transfers = self.transfers.write();
        transfers.retain(|_, t| {
            matches!(
                t.status,
                TransferStatus::Pending | TransferStatus::InProgress | TransferStatus::Offered
            )
        });
    }
}

impl Default for TransferManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Global transfer manager
static TRANSFER_MANAGER: once_cell::sync::Lazy<Arc<TransferManager>> =
    once_cell::sync::Lazy::new(|| Arc::new(TransferManager::new()));

/// Get the global transfer manager
pub fn get_transfer_manager() -> Arc<TransferManager> {
    TRANSFER_MANAGER.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_file_info_from_path() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "Hello, World!").unwrap();

        let info = FileInfo::from_path(&file_path).unwrap();
        assert_eq!(info.name, "test.txt");
        assert_eq!(info.size, 13);
        assert!(!info.checksum.is_empty());
    }

    #[test]
    fn test_file_sender_chunks() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.bin");

        // Create a file larger than chunk size
        let data: Vec<u8> = (0..CHUNK_SIZE * 2 + 1000)
            .map(|i| (i % 256) as u8)
            .collect();
        std::fs::write(&file_path, &data).unwrap();

        let mut sender = FileSender::new(&file_path).unwrap();
        assert_eq!(sender.chunk_count(), 3);

        // Read first chunk
        let chunk = sender.get_chunk(0).unwrap();
        assert_eq!(chunk.len(), CHUNK_SIZE);
        assert_eq!(&chunk[..], &data[..CHUNK_SIZE]);

        // Read second chunk
        let chunk = sender.get_chunk(CHUNK_SIZE as u64).unwrap();
        assert_eq!(chunk.len(), CHUNK_SIZE);

        // Read last chunk
        let chunk = sender.get_chunk((CHUNK_SIZE * 2) as u64).unwrap();
        assert_eq!(chunk.len(), 1000);
    }

    #[test]
    fn test_file_receiver() {
        let dir = tempdir().unwrap();
        let src_path = dir.path().join("source.bin");
        let dst_path = dir.path().join("dest.bin");

        // Create source file
        let data: Vec<u8> = (0..CHUNK_SIZE + 500)
            .map(|i| (i % 256) as u8)
            .collect();
        std::fs::write(&src_path, &data).unwrap();

        let info = FileInfo::from_path(&src_path).unwrap();
        let mut receiver = FileReceiver::new(info, &dst_path).unwrap();

        // Write chunks
        receiver.write_chunk(0, &data[..CHUNK_SIZE]).unwrap();
        receiver.write_chunk(CHUNK_SIZE as u64, &data[CHUNK_SIZE..]).unwrap();

        assert!(receiver.is_complete());
        assert!(receiver.verify().unwrap());
    }
}
