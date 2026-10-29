use anyhow::Result;
use sui_sdk::types::transaction::Argument;

use crate::library::swap_with_route::Swapper;

/// Legacy DeepBook V2 at 0xdee9 is deprecated.
/// Fail closed instead of constructing a transaction with unsupported Move calls.
pub async fn swap<'a>(_swapper: &mut Swapper<'a>) -> Result<Argument> {
    anyhow::bail!(
        "DeepBook V2 (0xdee9) is deprecated; refusing to build this swap until a supported implementation is verified"
    )
}
