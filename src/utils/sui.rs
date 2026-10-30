use crate::consts::{SUI_FULL_TYPE, SUI_TYPE};
use crate::library::get_coin_object_ids_by_amount::get_coin_object_ids_by_amount;
use anyhow::{Result, anyhow};
use std::ops::{Deref, DerefMut};
use std::str::FromStr;
use sui_sdk::rpc_types::SuiObjectDataOptions;
use sui_sdk::types::base_types::ObjectRef;
use sui_sdk::types::object::OBJECT_START_VERSION;
use sui_sdk::types::transaction::TransactionKind;
use sui_sdk::types::type_input::TypeInput;
use sui_sdk::types::{SUI_CLOCK_OBJECT_ID, SUI_CLOCK_OBJECT_SHARED_VERSION};
use sui_sdk::{
    SuiClient,
    rpc_types::{Coin, SuiObjectResponse, SuiObjectResponseQuery},
    types::{
        Identifier, TypeTag,
        base_types::{ObjectID, SuiAddress},
        programmable_transaction_builder::ProgrammableTransactionBuilder,
        transaction::{Argument, Command, ObjectArg},
    },
};

#[derive(Debug, Clone)]
pub struct DataPage<T> {
    pub data: Vec<T>,
    pub next_cursor: Option<String>,
    pub has_next_page: bool,
}

#[derive(Debug, Clone)]
pub struct PageQuery {
    pub cursor: Option<String>,
    pub limit: Option<u64>,
}

pub enum PaginationArgs {
    All,
    Query(PageQuery),
}

pub struct Ptb(pub ProgrammableTransactionBuilder);

impl Deref for Ptb {
    type Target = ProgrammableTransactionBuilder;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Ptb {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Ptb {
    pub fn new() -> Self {
        Ptb(ProgrammableTransactionBuilder::new())
    }

    pub fn gas(&self) -> Argument {
        Argument::GasCoin
    }

    pub fn complete(self) -> TransactionKind {
        TransactionKind::ProgrammableTransaction(self.0.finish())
    }

    pub fn get_sui_coin(&mut self, amount: u64) -> Result<Argument> {
        let amount = self.pure(amount)?;
        let coin = self.command(Command::SplitCoins(Argument::GasCoin, vec![amount]));

        Ok(coin)
    }

    pub fn merge_coins(&mut self, mut coin_objects: Vec<Argument>) -> Argument {
        if coin_objects.len() == 1 {
            return coin_objects[0];
        }

        let first_coin = coin_objects.remove(0);
        self.command(Command::MergeCoins(first_coin, coin_objects));

        first_coin
    }

    pub fn split_coins(
        &mut self,
        coin_object: Argument,
        split_amounts: &[u64],
    ) -> Result<Argument> {
        let split_amounts = split_amounts
            .iter()
            .map(|item| self.pure(item))
            .collect::<Result<Vec<Argument>>>()?;

        Ok(self.command(Command::SplitCoins(coin_object, split_amounts)))
    }

    pub fn transfer_objects(
        &mut self,
        object_refs: Vec<Argument>,
        recipient: Argument,
    ) -> Result<()> {
        self.command(Command::TransferObjects(object_refs, recipient));

        return Ok(());
    }

    pub fn coin_value(&mut self, coin_type: String, coin_object: Argument) -> Result<Argument> {
        let package = ObjectID::from_str("0x2")?;
        let module = Identifier::from_str("coin")?;
        let function = Identifier::from_str("value")?;
        let coin_type = TypeTag::from_str(&coin_type)?;

        let value = self.command(Command::move_call(
            package,
            module,
            function,
            vec![coin_type],
            vec![coin_object],
        ));

        Ok(value)
    }

    pub fn move_call(
        &mut self,
        package: &str,
        module: &str,
        function: &str,
        type_arguments: Vec<TypeTag>,
        arguments: Vec<Argument>,
    ) -> Result<Argument> {
        Ok(self.command(Command::move_call(
            ObjectID::from_str(package)?,
            Identifier::from_str(module)?,
            Identifier::from_str(function)?,
            type_arguments,
            arguments,
        )))
    }

    pub fn make_move_vec(
        &mut self,
        type_input: TypeTag,
        arguments: Vec<Argument>,
    ) -> Result<Argument> {
        Ok(self.command(Command::MakeMoveVec(
            Some(TypeInput::from(type_input)),
            arguments,
        )))
    }

    pub fn get_exact_coin_by_amount(
        &mut self,
        coin_type: &str,
        coins: Vec<Coin>,
        amount: u64,
    ) -> Result<Argument> {
        if is_sui_coin(coin_type) {
            let amount = self.pure(amount)?;
            let coin = self.command(Command::SplitCoins(Argument::GasCoin, vec![amount]));
            return Ok(coin);
        } else {
            let coins_x = get_coins_greater_than_amount(amount, &coins)?;

            if coins_x.len() > 1 {
                let first_coin = self.obj(ObjectArg::ImmOrOwnedObject(coins_x[0].object_ref()))?;
                let other_coins = coins_x[1..]
                    .iter()
                    .map(|c| self.obj(ObjectArg::ImmOrOwnedObject(c.object_ref())))
                    .collect::<Result<Vec<_>>>()?;

                self.command(Command::MergeCoins(first_coin, other_coins));
            }

            let amount_arg = self.pure(amount)?;
            let master_coin = self.obj(ObjectArg::ImmOrOwnedObject(coins_x[0].object_ref()))?;
            let coin = self.command(Command::SplitCoins(master_coin, vec![amount_arg]));

            Ok(coin)
        }
    }

    pub fn zero_balance(&mut self, coin_type: &str) -> Result<Argument> {
        Ok(self.command(Command::move_call(
            ObjectID::from_str("0x2")?,
            Identifier::from_str("balance")?,
            Identifier::from_str("zero")?,
            vec![TypeTag::from_str(coin_type)?],
            vec![],
        )))
    }

    pub fn zero_coin(&mut self, coin_type: &str) -> Result<Argument> {
        Ok(self.command(Command::move_call(
            ObjectID::from_str("0x2")?,
            Identifier::from_str("coin")?,
            Identifier::from_str("zero")?,
            vec![TypeTag::from_str(coin_type)?],
            vec![],
        )))
    }

    pub fn coin_into_balance(
        &mut self,
        coin_type: &str,
        coin_object: Argument,
    ) -> Result<Argument> {
        Ok(self.command(Command::move_call(
            ObjectID::from_str("0x2")?,
            Identifier::from_str("coin")?,
            Identifier::from_str("into_balance")?,
            vec![TypeTag::from_str(coin_type)?],
            vec![coin_object],
        )))
    }

    pub fn coin_from_balance(&mut self, coin_type: &str, balance: Argument) -> Result<Argument> {
        Ok(self.command(Command::move_call(
            ObjectID::from_str("0x2")?,
            Identifier::from_str("coin")?,
            Identifier::from_str("from_balance")?,
            vec![TypeTag::from_str(coin_type)?],
            vec![balance],
        )))
    }

    pub fn transfer_or_destroy_zero_coin(
        &mut self,
        coin_type: &str,
        coin: Argument,
        to: Option<SuiAddress>,
    ) -> Result<()> {
        let target = if to.is_some() {
            "send_coin"
        } else {
            "transfer_coin_to_sender"
        };

        let mut args = vec![coin];
        if let Some(addr) = to {
            args.push(self.pure(addr.to_string())?);
        }

        self.command(Command::move_call(
            ObjectID::from_str(
                "0x6f5e582ede61fe5395b50c4a449ec11479a54d7ff8e0158247adfda60d98970b",
            )?,
            Identifier::from_str("utils")?,
            Identifier::from_str(target)?,
            vec![TypeTag::from_str(coin_type)?],
            args,
        ));

        Ok(())
    }

    pub fn coin_to_arg(&mut self, coin: &Coin) -> Result<Argument> {
        self.obj(ObjectArg::ImmOrOwnedObject(coin.object_ref()))
    }

    pub fn coins_to_args(&mut self, coins: &[Coin]) -> Result<Vec<Argument>> {
        coins
            .iter()
            .map(|item| self.obj(ObjectArg::ImmOrOwnedObject(item.object_ref())))
            .collect::<Result<Vec<Argument>>>()
    }

    pub async fn get_split_coin_for_tx(
        &mut self,
        client: &SuiClient,
        account: SuiAddress,
        amount: u64,
        splits: &[u64],
        coin_type: &str,
        _inspect_transaction: Option<bool>,
    ) -> Result<Argument> {
        // SUI input must have identical construction in simulation and final builds.
        // GasCoin is the transaction's selected gas payment object; wallet aggregate
        // balance cannot prove that this specific object covers input plus gas.
        if is_sui_coin(coin_type) {
            if amount == 0 {
                return Err(anyhow!("Requested SUI amount must be greater than zero"));
            }
            validate_sui_split_amounts(amount, splits)?;
            return self.split_coins(self.gas(), splits);
        }

        let (_object_ids, coins, _balance) =
            get_coin_object_ids_by_amount(client, account, amount, coin_type).await?;

        if let Some(main_coin) = coins.first() {
            if coins.len() > 1 {
                let coins = self.coins_to_args(&coins)?;
                self.merge_coins(coins);
            }

            // split correct amount to swap
            let coin = self.coin_to_arg(main_coin)?;
            let coins_arg = self.split_coins(coin, splits)?;
            return Ok(coins_arg);
        }

        Err(anyhow!("No valid coin object IDs found"))
    }

    pub fn clock(&mut self) -> Result<Argument> {
        let obj_arg = ObjectArg::SharedObject {
            id: SUI_CLOCK_OBJECT_ID,
            initial_shared_version: SUI_CLOCK_OBJECT_SHARED_VERSION,
            mutable: false,
        };

        self.obj(obj_arg)
    }
}

pub fn get_coins_greater_than_amount(amount: u64, coins: &Vec<Coin>) -> Result<Vec<Coin>> {
    let mut coins_with_balance: Vec<Coin> = Vec::new();
    let mut collected_amount = 0u64;

    for coin in coins {
        if collected_amount < amount && !coins_with_balance.contains(coin) {
            collected_amount += coin.balance;
            coins_with_balance.push(coin.clone());
        }
        if coin.balance == 0 && !coins_with_balance.contains(coin) {
            coins_with_balance.push(coin.clone());
        }
    }

    if collected_amount >= amount {
        Ok(coins_with_balance)
    } else {
        Err(anyhow::anyhow!("Insufficient balance"))
    }
}

pub async fn get_owned_objects_by_page(
    sui_client: SuiClient,
    owner: SuiAddress,
    query: SuiObjectResponseQuery,
    pagination_args: PaginationArgs,
) -> Result<DataPage<SuiObjectResponse>> {
    let mut result = Vec::new();
    let mut has_next_page = true;
    let query_all = matches!(pagination_args, PaginationArgs::All);
    let mut next_cursor = match &pagination_args {
        PaginationArgs::All => None,
        PaginationArgs::Query(q) => q.cursor.clone(),
    };

    while query_all && has_next_page {
        let res = sui_client
            .read_api()
            .get_owned_objects(
                owner,
                Some(query.clone()),
                next_cursor.clone().map(|c| ObjectID::from_str(&c).unwrap()),
                match &pagination_args {
                    PaginationArgs::All => None,
                    PaginationArgs::Query(q) => q.limit.map(|l| l as usize),
                },
            )
            .await?;

        if !res.data.is_empty() {
            result.extend(res.data);
            has_next_page = res.has_next_page;
            next_cursor = res.next_cursor.map(|c| c.to_string());
        } else {
            has_next_page = false;
        }
    }

    Ok(DataPage {
        data: result,
        next_cursor,
        has_next_page,
    })
}

fn validate_sui_split_amounts(amount: u64, splits: &[u64]) -> Result<()> {
    let split_total = splits
        .iter()
        .try_fold(0u64, |total, split| total.checked_add(*split))
        .ok_or_else(|| anyhow!("SUI split amount overflow"))?;
    if split_total != amount {
        return Err(anyhow!(
            "SUI split amounts ({split_total}) do not equal requested amount ({amount})"
        ));
    }
    Ok(())
}

fn is_sui_coin(coin_type: &str) -> bool {
    coin_type == SUI_TYPE || coin_type == SUI_FULL_TYPE
}

#[async_trait::async_trait]
pub trait ObjectRefFetcher {
    async fn object_ref(&self, object_id: &str) -> Result<ObjectRef>;

    async fn owned_obj(&self, object_id: &str) -> Result<ObjectArg>;

    async fn shared_obj_mut(&self, object_id: &str) -> Result<ObjectArg>;

    async fn shared_obj_imut(&self, object_id: &str) -> Result<ObjectArg>;
}

#[async_trait::async_trait]
impl ObjectRefFetcher for SuiClient {
    async fn object_ref(&self, object_str: &str) -> Result<ObjectRef> {
        let object_id = ObjectID::from_str(object_str)?;

        let object: SuiObjectResponse = self
            .read_api()
            .get_object_with_options(object_id, SuiObjectDataOptions::default())
            .await?;

        if let Some(error) = object.error {
            return Err(anyhow!(error));
        }

        if let Some(data) = object.data {
            Ok(data.object_ref())
        } else {
            Err(anyhow!("No data found for object {:?}", object_id))
        }
    }

    async fn owned_obj(&self, object_id: &str) -> Result<ObjectArg> {
        Ok(self.object_ref(object_id).await?.owned_obj())
    }

    async fn shared_obj_mut(&self, object_id: &str) -> Result<ObjectArg> {
        Ok(self.object_ref(object_id).await?.shared_obj(true))
    }

    async fn shared_obj_imut(&self, object_id: &str) -> Result<ObjectArg> {
        Ok(self.object_ref(object_id).await?.shared_obj(false))
    }
}

pub trait ObjectArgExt {
    fn owned_obj(&self) -> ObjectArg;
    fn shared_obj(&self, is_mut: bool) -> ObjectArg;
}

impl ObjectArgExt for ObjectRef {
    fn owned_obj(&self) -> ObjectArg {
        ObjectArg::ImmOrOwnedObject(*self)
    }

    fn shared_obj(&self, is_mut: bool) -> ObjectArg {
        ObjectArg::SharedObject {
            id: self.0,
            initial_shared_version: OBJECT_START_VERSION,
            mutable: is_mut,
        }
    }
}

pub trait ArgumentExt {
    fn get_slice(&self, idx: u16) -> Result<Argument>;

    fn split(&self, idx: u16) -> Result<Vec<Argument>>;
}

impl ArgumentExt for Argument {
    fn get_slice(&self, idx: u16) -> Result<Argument> {
        match self {
            Argument::Result(idx_) => Ok(Argument::NestedResult(*idx_, idx)),
            _ => Err(anyhow!("Expecting Argument to be a Result")),
        }
    }

    fn split(&self, len: u16) -> Result<Vec<Argument>> {
        let mut args = vec![];

        for i in 0..len {
            args.push(self.get_slice(i)?);
        }

        Ok(args)
    }
}

#[macro_export]
macro_rules! destruct {
    // Case for a tuple with 1 element
    (1, $vec:expr) => {{
        let v = &$vec;
        (v[0],)
    }};

    // Case for a tuple with 2 elements
    (2, $vec:expr) => {{
        let v = &$vec;
        (v[0], v[1])
    }};

    // Case for a tuple with 3 elements
    (3, $vec:expr) => {{
        let v = &$vec;
        (v[0], v[1], v[2])
    }};

    // Case for a tuple with 4 elements
    (4, $vec:expr) => {{
        let v = &$vec;
        (v[0], v[1], v[2], v[3])
    }};

    // Case for a tuple with 5 elements
    (5, $vec:expr) => {{
        let v = &$vec;
        (v[0], v[1], v[2], v[3], v[4])
    }};

    // Case for a tuple with 6 elements
    (6, $vec:expr) => {{
        let v = &$vec;
        (v[0], v[1], v[2], v[3], v[4], v[5])
    }};

    // Case for a tuple with 7 elements
    (7, $vec:expr) => {{
        let v = &$vec;
        (v[0], v[1], v[2], v[3], v[4], v[5], v[6])
    }};

    // Case for a tuple with 8 elements
    (8, $vec:expr) => {{
        let v = &$vec;
        (v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7])
    }};

    // Case for a tuple with 9 elements
    (9, $vec:expr) => {{
        let v = &$vec;
        (v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7], v[8])
    }};

    // Case for a tuple with 10 elements
    (10, $vec:expr) => {{
        let v = &$vec;
        (v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7], v[8], v[9])
    }};

    // Default case for unsupported sizes (you could also panic if needed)
    (_size:expr, $vec:expr) => {
        compile_error!("Tuple size too large!");
    };
}

#[cfg(test)]
mod sui_input_tests {
    use super::validate_sui_split_amounts;

    #[test]
    fn accepts_route_splits_matching_requested_sui_amount() {
        assert!(validate_sui_split_amounts(12, &[5, 7]).is_ok());
    }

    #[test]
    fn rejects_route_splits_that_do_not_match_requested_sui_amount() {
        assert!(validate_sui_split_amounts(12, &[5, 6]).is_err());
    }

    #[test]
    fn rejects_sui_split_sum_overflow() {
        assert!(validate_sui_split_amounts(u64::MAX, &[u64::MAX, 1]).is_err());
    }
}
