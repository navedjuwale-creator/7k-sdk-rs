use anyhow::{Context, Result, anyhow, bail};
use std::{collections::HashSet, time::Duration};
use sui_sdk::{
    rpc_types::Coin,
    types::base_types::{ObjectID, SuiAddress},
};

use crate::client::get_sui_client;

// Production inventory bounds. Reaching any bound before the server reports a
// natural end is an error: callers must never receive a successful partial inventory.
const COIN_PAGE_SIZE: usize = 1_000;
const MAX_COIN_PAGES: usize = 100;
const MAX_COIN_OBJECTS: usize = 10_000;
const COIN_INVENTORY_DEADLINE: Duration = Duration::from_secs(30);

fn order_coins(coins: &mut [Coin]) {
    coins.sort_by(|left, right| right.balance.cmp(&left.balance));
}

fn select_coin_indices(balances: &[u64], amount: u64) -> Result<(Vec<usize>, u64)> {
    if amount == 0 {
        bail!("Requested coin amount must be greater than zero");
    }

    let mut selected = Vec::new();
    let mut selected_balance = 0u64;

    for (index, balance) in balances.iter().enumerate() {
        selected_balance = selected_balance
            .checked_add(*balance)
            .ok_or_else(|| anyhow!("Coin selection balance overflow"))?;
        selected.push(index);

        if selected_balance >= amount {
            return Ok((selected, selected_balance));
        }
    }

    bail!(
        "Insufficient balance: complete inventory contains {selected_balance}, requested {amount}"
    )
}

pub async fn get_coin_object_ids_by_amount(
    address: SuiAddress,
    amount: u64,
    coin_type: &str,
) -> Result<(Vec<ObjectID>, Vec<Coin>, u64)> {
    if amount == 0 {
        bail!("Requested coin amount must be greater than zero");
    }

    let client = get_sui_client().await.context("Create Sui RPC client")?;
    let deadline = tokio::time::Instant::now() + COIN_INVENTORY_DEADLINE;
    let mut coin_balances: Vec<Coin> = Vec::new();
    let mut next_cursor = None;
    let mut seen_cursors = HashSet::new();
    let mut total_balance = 0u64;

    for page_number in 1..=MAX_COIN_PAGES {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            bail!("Coin inventory deadline exceeded; inventory is incomplete");
        }

        let page = tokio::time::timeout(
            remaining,
            client.coin_read_api().get_coins(
                address,
                Some(coin_type.to_string()),
                next_cursor.take(),
                Some(COIN_PAGE_SIZE),
            ),
        )
        .await
        .map_err(|_| anyhow!("Coin inventory deadline exceeded; inventory is incomplete"))?
        .with_context(|| format!("Coin inventory RPC failed on page {page_number}"))?;

        if page.data.len() > COIN_PAGE_SIZE {
            bail!(
                "Coin inventory page {page_number} exceeded requested page size; inventory is incomplete"
            );
        }

        let new_total = coin_balances
            .len()
            .checked_add(page.data.len())
            .ok_or_else(|| anyhow!("Coin inventory object count overflow"))?;
        if new_total > MAX_COIN_OBJECTS {
            bail!(
                "Coin inventory exceeded the {MAX_COIN_OBJECTS}-object limit; inventory is incomplete"
            );
        }

        for coin in &page.data {
            total_balance = total_balance
                .checked_add(coin.balance)
                .ok_or_else(|| anyhow!("Total coin inventory balance overflow"))?;
        }
        coin_balances.extend(page.data);

        if !page.has_next_page {
            break;
        }

        let cursor = page.next_cursor.ok_or_else(|| {
            anyhow!(
                "Coin inventory page {page_number} reports another page but supplies no cursor; inventory is incomplete"
            )
        })?;
        let cursor_key = cursor.to_string();
        if !seen_cursors.insert(cursor_key) {
            bail!("Coin inventory repeated a pagination cursor; inventory is incomplete");
        }
        next_cursor = Some(cursor);

        if page_number == MAX_COIN_PAGES {
            bail!(
                "Coin inventory exceeded the {MAX_COIN_PAGES}-page limit; inventory is incomplete"
            );
        }
    }

    // Inventory is complete only after the RPC reports its natural end.
    // The total is checked above even though selection uses only the coins needed.
    let _complete_inventory_balance = total_balance;
    order_coins(&mut coin_balances);

    let balances: Vec<u64> = coin_balances.iter().map(|coin| coin.balance).collect();
    let (selected_indices, selected_balance) = select_coin_indices(&balances, amount)?;

    let selected_coins: Vec<Coin> = selected_indices
        .into_iter()
        .map(|index| coin_balances[index].clone())
        .collect();
    let object_ids = selected_coins
        .iter()
        .map(|coin| coin.coin_object_id)
        .collect();

    Ok((object_ids, selected_coins, selected_balance))
}

#[cfg(test)]
mod tests {
    use super::select_coin_indices;

    #[test]
    fn rejects_zero_amount() {
        assert!(select_coin_indices(&[10, 20], 0).is_err());
    }

    #[test]
    fn selects_only_enough_coins() {
        let (indices, balance) = select_coin_indices(&[8, 5, 2], 10).unwrap();
        assert_eq!(indices, vec![0, 1]);
        assert_eq!(balance, 13);
    }

    #[test]
    fn reports_insufficient_complete_inventory() {
        let error = select_coin_indices(&[3, 4], 10).unwrap_err();
        assert!(error.to_string().contains("Insufficient balance"));
    }

    #[test]
    fn rejects_selection_overflow() {
        assert!(select_coin_indices(&[u64::MAX, 1], u64::MAX).is_err());
    }

    #[test]
    fn empty_inventory_is_insufficient() {
        assert!(select_coin_indices(&[], 1).is_err());
    }
}
