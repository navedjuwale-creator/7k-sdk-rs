use anyhow::{bail, Result};
use sui_sdk::{
    rpc_types::Coin,
    types::base_types::{ObjectID, SuiAddress},
    SuiClient,
};

use crate::client::get_sui_client;

fn order_by_balance<T, F>(array: &mut [T], sort_by: &str, balance: F)
where
    F: Fn(&T) -> u64,
{
    array.sort_by(|left, right| {
        let ordering = balance(left).cmp(&balance(right));
        if sort_by == "desc" {
            ordering.reverse()
        } else {
            ordering
        }
    });
}

fn order_coins(array: &mut [Coin], sort_by: &str) {
    order_by_balance(array, sort_by, |coin| coin.balance);
}

pub async fn get_coin_object_ids_by_amount(
    address: SuiAddress,
    amount: u64,
    coin_type: &str,
) -> Result<(Vec<ObjectID>, Vec<Coin>, u64)> {
    ensure_positive_amount(amount)?;
    let client = get_sui_client().await?;
    get_coin_object_ids_by_amount_with_client(&client, address, amount, coin_type).await
}

pub async fn get_coin_object_ids_by_amount_with_client(
    client: &SuiClient,
    address: SuiAddress,
    amount: u64,
    coin_type: &str,
) -> Result<(Vec<ObjectID>, Vec<Coin>, u64)> {
    ensure_positive_amount(amount)?;

    let mut coin_balances: Vec<Coin> = Vec::new();
    let mut has_next_page = true;
    let mut next_cursor: Option<ObjectID> = None;
    let mut used_cursors = Vec::new();

    while has_next_page {
        let page_cursor = next_cursor.take();

        if let Some(cursor) = page_cursor.as_ref() {
            if used_cursors.contains(cursor) {
                bail!("Sui coin pagination returned a repeated cursor");
            }
            used_cursors.push(cursor.clone());
        }

        let coins = client
            .coin_read_api()
            .get_coins(address, Some(coin_type.to_string()), page_cursor, None)
            .await?;

        coin_balances.extend(coins.data);
        has_next_page = coins.has_next_page;
        next_cursor = coins.next_cursor;

        validate_next_cursor(has_next_page, next_cursor.as_ref(), &used_cursors)?;
    }

    order_coins(&mut coin_balances, "desc");

    let mut balance: u64 = 0;
    let mut object_ids = Vec::new();
    let mut object_coins = Vec::new();

    for coin in &coin_balances {
        balance = checked_add_balance(balance, coin.balance)?;
        object_ids.push(coin.coin_object_id.clone());
        object_coins.push(coin.clone());
        if balance >= amount {
            break;
        }
    }

    ensure_sufficient_balance(amount, balance)?;

    Ok((object_ids, object_coins, balance))
}

fn validate_next_cursor<T: PartialEq>(
    has_next_page: bool,
    next_cursor: Option<&T>,
    used_cursors: &[T],
) -> anyhow::Result<()> {
    if !has_next_page {
        return Ok(());
    }

    let cursor = next_cursor.ok_or_else(|| {
        anyhow::anyhow!("Sui coin pagination reports another page but provides no cursor")
    })?;

    if used_cursors.contains(cursor) {
        bail!("Sui coin pagination returned a repeated cursor");
    }

    Ok(())
}

fn checked_add_balance(total: u64, next: u64) -> anyhow::Result<u64> {
    total
        .checked_add(next)
        .ok_or_else(|| anyhow::anyhow!("Total coin balance exceeds u64::MAX"))
}

fn ensure_sufficient_balance(requested: u64, available: u64) -> anyhow::Result<()> {
    if available < requested {
        anyhow::bail!("Insufficient coin balance: requested {requested}, available {available}");
    }
    Ok(())
}

fn ensure_positive_amount(amount: u64) -> anyhow::Result<()> {
    if amount == 0 {
        anyhow::bail!("Requested coin amount must be greater than zero");
    }
    Ok(())
}

#[cfg(test)]
mod coin_order_tests {
    use super::order_by_balance;

    #[test]
    fn pagination_rejects_missing_cursor_when_another_page_exists() {
        assert!(super::validate_next_cursor::<u8>(true, None, &[]).is_err());
    }

    #[test]
    fn pagination_rejects_repeated_cursor() {
        let used = vec![1u8, 2u8];
        assert!(super::validate_next_cursor(true, Some(&2u8), &used).is_err());
    }

    #[test]
    fn pagination_accepts_new_cursor() {
        let used = vec![1u8];
        assert!(super::validate_next_cursor(true, Some(&2u8), &used).is_ok());
    }

    #[test]
    fn pagination_accepts_no_cursor_on_final_page() {
        assert!(super::validate_next_cursor::<u8>(false, None, &[]).is_ok());
    }

    #[test]
    fn checked_balance_addition_rejects_overflow() {
        assert!(super::checked_add_balance(u64::MAX, 1).is_err());
    }

    #[test]
    fn checked_balance_addition_accepts_valid_sum() {
        assert_eq!(super::checked_add_balance(40, 2).unwrap(), 42);
    }

    #[test]
    fn insufficient_balance_is_rejected() {
        let err = super::ensure_sufficient_balance(100, 80).unwrap_err();
        assert!(err.to_string().contains("requested 100, available 80"));
    }

    #[test]
    fn exact_balance_is_accepted() {
        assert!(super::ensure_sufficient_balance(100, 100).is_ok());
    }

    #[test]
    fn excess_balance_is_accepted() {
        assert!(super::ensure_sufficient_balance(100, 120).is_ok());
    }

    #[test]
    fn zero_amount_is_rejected() {
        assert!(super::ensure_positive_amount(0).is_err());
    }

    #[test]
    fn positive_amount_is_accepted() {
        assert!(super::ensure_positive_amount(1).is_ok());
    }

    #[test]
    fn maximum_u64_amount_is_accepted_by_validator() {
        assert!(super::ensure_positive_amount(u64::MAX).is_ok());
    }

    #[test]
    fn empty_list_is_safe() {
        let mut values: Vec<(u8, u64)> = vec![];
        order_by_balance(&mut values, "desc", |item| item.1);
        assert!(values.is_empty());
    }

    #[test]
    fn single_item_is_unchanged() {
        let mut values = vec![(1u8, 42u64)];
        order_by_balance(&mut values, "desc", |item| item.1);
        assert_eq!(values, vec![(1, 42)]);
    }

    #[test]
    fn descending_order_is_correct() {
        let mut values = vec![(1u8, 4u64), (2, 12), (3, 7)];
        order_by_balance(&mut values, "desc", |item| item.1);
        assert_eq!(
            values.iter().map(|item| item.1).collect::<Vec<_>>(),
            vec![12, 7, 4]
        );
    }

    #[test]
    fn ascending_order_is_correct() {
        let mut values = vec![(1u8, 4u64), (2, 12), (3, 7)];
        order_by_balance(&mut values, "asc", |item| item.1);
        assert_eq!(
            values.iter().map(|item| item.1).collect::<Vec<_>>(),
            vec![4, 7, 12]
        );
    }
}
