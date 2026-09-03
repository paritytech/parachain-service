//! `display-chain`: the most recent blocks, newest first.

use cumulus_jam_interface::JamChainSource;

use crate::format::hex;
use cumulus_jam_rpc_interface::JamRpcInterface;

/// Print the last `count` blocks of the best or finalized chain.
///
/// There is no "give me the last N blocks" RPC, so this walks parents from the tip. Note the
/// number shown is the *slot*, which is what `BlockDesc` carries; slots can be empty, so it is not
/// a block height.
pub async fn display(jam: &JamRpcInterface, count: usize, finalized: bool) -> Result<(), String> {
	let mut block = if finalized {
		jam.finalized_block().await.map_err(|e| format!("finalized block: {e}"))?
	} else {
		jam.best_block().await.map_err(|e| format!("best block: {e}"))?
	};

	println!("{:<12}  {}", "slot", "header hash");
	for _ in 0..count {
		println!("{:<12}  0x{}", block.slot, hex(&*block.header_hash));
		let parent = jam
			.parent(block.header_hash)
			.await
			.map_err(|e| format!("parent of {:?}: {e}", block.header_hash))?;
		// Genesis is its own parent; stop rather than printing it forever.
		if parent.header_hash == block.header_hash {
			break;
		}
		block = parent;
	}
	Ok(())
}
