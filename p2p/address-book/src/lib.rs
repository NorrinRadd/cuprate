//! Cuprate Address Book
//!
//! This module holds the logic for persistent peer storage.
//! Cuprates address book is modeled as a [`tower::Service`]
//! The request is [`AddressBookRequest`](cuprate_p2p_core::services::AddressBookRequest) and the response is
//! [`AddressBookResponse`](cuprate_p2p_core::services::AddressBookResponse).
//!
//! Cuprate, like monerod, actually has multiple address books, one
//! for each [`NetworkZone`]. This is to reduce the possibility of
//! clear net peers getting linked to their dark counterparts
//! and so peers will only get told about peers they can
//! connect to.
use std::{io::ErrorKind, path::PathBuf, time::Duration};
use std::collections::HashMap;
use std::io::BufRead;
use std::path::Path;
use std::time::Instant;
use cuprate_p2p_core::{NetZoneAddress, NetworkZone};

mod book;
mod peer_list;
mod store;

/// The address book config.
#[derive(Debug, Clone)]
pub struct AddressBookConfig {
    /// The maximum number of white peers in the peer list.
    ///
    /// White peers are peers we have connected to before.
    pub max_white_list_length: usize,
    /// The maximum number of gray peers in the peer list.
    ///
    /// Gray peers are peers we are yet to make a connection to.
    pub max_gray_list_length: usize,
    /// The location to store the peer store files.
    pub peer_store_directory: PathBuf,

    pub ban_list_path: Option<PathBuf>,
    /// The amount of time between saving the address book to disk.
    pub peer_save_period: Duration,
}

/// Possible errors when dealing with the address book.
/// This is boxed when returning an error in the [`tower::Service`].
#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum AddressBookError {
    /// The peer is already connected.
    #[error("Peer is already connected")]
    PeerAlreadyConnected,
    /// The peer is not in the address book for this zone.
    #[error("Peer was not found in book")]
    PeerNotFound,
    /// Immutable peer data was changed.
    #[error("Immutable peer data was changed: {0}")]
    PeersDataChanged(&'static str),
    /// The peer is banned.
    #[error("The peer is banned")]
    PeerIsBanned,
    /// The channel to the address book has closed unexpectedly.
    #[error("The address books channel has closed.")]
    AddressBooksChannelClosed,
    /// The address book task has exited.
    #[error("The address book task has exited.")]
    AddressBookTaskExited,
}

/// Initializes the P2P address book for a specific network zone.
pub async fn init_address_book<Z: BorshNetworkZone>(
    cfg: AddressBookConfig,
) -> Result<book::AddressBook<Z>, std::io::Error> {
    let (white_list, gray_list) = match store::read_peers_from_disk::<Z>(&cfg).await {
        Ok(res) => res,
        Err(e) if e.kind() == ErrorKind::NotFound => (vec![], vec![]),
        Err(e) => {
            tracing::error!("Failed to open peer list, {}", e);
            panic!("{e}");
        }
    };

    let peers_to_ban = if let Some(ban_list_path) = &cfg.ban_list_path {
        read_ban_list::<Z>(ban_list_path).await
    } else {
        vec![]
    };

    let mut address_book = book::AddressBook::<Z>::new(cfg, white_list, gray_list, Vec::new());

    for ban in peers_to_ban {
        tracing::info!("Banning peer {}", ban);
        address_book.ban_peer(ban, Duration::from_secs(60 * 60 * 24 * 365));
    }

    Ok(address_book)
}

async fn read_ban_list<Z: BorshNetworkZone>(path: &Path) -> Vec<Z::Addr> {
    let file = tokio::fs::read_to_string(path).await.unwrap();

    let mut bans = vec![];
    for line in file.lines() {
        let mut ban_ids = <Z::Addr as NetZoneAddress>::read_ban_line(line);
        bans.append(&mut ban_ids);
    }

    bans
}

use sealed::BorshNetworkZone;
mod sealed {
    use super::*;

    /// An internal trait for the address book for a [`NetworkZone`] that adds the requirement of [`borsh`] traits
    /// onto the network address.
    pub trait BorshNetworkZone: NetworkZone<Addr = Self::BorshAddr> {
        type BorshAddr: NetZoneAddress + borsh::BorshDeserialize + borsh::BorshSerialize;
    }

    impl<T: NetworkZone> BorshNetworkZone for T
    where
        T::Addr: borsh::BorshDeserialize + borsh::BorshSerialize,
    {
        type BorshAddr = T::Addr;
    }
}
