use cuprate_blockchain::service::BlockchainReadHandle;
use cuprate_p2p::block_downloader::ChainEntry;
use cuprate_p2p_core::NetworkZone;
use cuprate_types::blockchain::{BlockchainReadRequest, BlockchainResponse};
use cuprate_types::Chain;
use std::collections::VecDeque;
use std::ops::Range;
use std::slice;
use tower::{Service, ServiceExt};
use blake3::Hasher;

static FAST_SYNC_HASHES: &[[u8; 32]] = unsafe {
    let bytes = include_bytes!("./fast_sync/fast_sync_hashes.bin");
    if bytes.len() % 32 != 0 {
        panic!()
    }

    slice::from_raw_parts(bytes.as_ptr().cast::<[u8; 32]>(), bytes.len() / 32)
};

const FAST_SYNC_BATCH_LEN: usize = 512;

static FAST_SYNC_TOP_HEIGHT: usize = FAST_SYNC_HASHES.len() * FAST_SYNC_BATCH_LEN;

pub async fn validate_entries<N: NetworkZone>(
    mut entries: VecDeque<ChainEntry<N>>,
    start_height: usize,
    blockchain_read_handle: &mut BlockchainReadHandle,
) -> Result<(VecDeque<ChainEntry<N>>, VecDeque<ChainEntry<N>>), tower::BoxError> {
    let hashes_start_height = (start_height / FAST_SYNC_BATCH_LEN) * FAST_SYNC_BATCH_LEN;
    let amount_of_hashes = entries.iter().map(|e| e.ids.len()).sum::<usize>();
    let last_height = amount_of_hashes + start_height - 1;

    let hashes_stop_height = (last_height / FAST_SYNC_BATCH_LEN) * FAST_SYNC_BATCH_LEN - 1;

    let mut hashes_stop_diff_last_height = last_height - hashes_stop_height;

    let mut unknown = VecDeque::new();

    while !entries.is_empty() {
        let back = entries.back_mut().unwrap();

        if back.ids.len() >= hashes_stop_diff_last_height {
            unknown.push_front(ChainEntry {
                ids: back
                    .ids
                    .drain((back.ids.len() - hashes_stop_diff_last_height)..)
                    .collect(),
                peer: back.peer.clone(),
                handle: back.handle.clone(),
            });

            break;
        }

        let back = entries.pop_back().unwrap();
        hashes_stop_diff_last_height -= back.ids.len();
        unknown.push_front(back);
    }

    let BlockchainResponse::BlockHashInRange(mut hashes) = blockchain_read_handle
        .ready()
        .await?
        .call(BlockchainReadRequest::BlockHashInRange(
            hashes_start_height..start_height,
            Chain::Main,
        ))
        .await?
    else {
        unreachable!()
    };


    let mut hasher = Hasher::default();
    for (i, hash) in hashes.iter().chain(entries.iter().flat_map(|e| e.ids.iter())).enumerate() {
        if (i + 1) % FAST_SYNC_BATCH_LEN == 0 {
            let got_hash = hasher.finalize();

            if got_hash != FAST_SYNC_HASHES[(i + 2 + hashes_start_height) / FAST_SYNC_BATCH_LEN ] {
                panic!("fast_sync_top_height: {FAST_SYNC_TOP_HEIGHT},  hashes_start_height: {hashes_start_height}, start_height: {start_height}, amount_of_hashes: {amount_of_hashes}, hashes_stop_height: {hashes_stop_height}, hashes_stop_diff_last_height: {hashes_stop_diff_last_height}");
                return Err("Hashes do not match".into());
            }
            hasher.reset();
        }

        hasher.update(hash);
    }

    let got_hash = hasher.finalize();

    if got_hash != FAST_SYNC_HASHES[last_height / FAST_SYNC_BATCH_LEN ] {
        panic!()
    }

    Ok((entries, unknown))
}
