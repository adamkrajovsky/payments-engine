use std::collections::HashMap;

use csv::{ReaderBuilder, Trim};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
enum Error {
    #[error("Account (id: {0}) is locked")]
    AccountLocked(u16),
    #[error("Transaction (id: {0}) does not have an amount")]
    MissingTxAmount(u32),
    #[error("Client does not have enough funds to perform the transaction (id: {0})")]
    NotEnoughFunds(u32),
    #[error("Transaction (id: {0}) does not exist")]
    TxDoesNotExist(u32),
    #[error("Transaction (id: {0}) is not under dispute")]
    TxNotUnderDispute(u32),
    #[error("Transaction (id: {0}) is already under dispute")]
    TxAlreadyUnderDispute(u32),
    #[error("Transaction (id: {0}) has an invalid amount")]
    TxInvalidAmount(u32),
    #[error("Transaction (id: {0}) cannot be disputed as it is not a deposit")]
    InvalidDispute(u32),
    #[error(
        "Client id of {0:?} does not match the client id of the original transaction (tx id: {1})"
    )]
    ClientIdMismatch(TxType, u32),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TxType {
    Deposit,
    Withdrawal,
    Dispute,
    Resolve,
    ChargeBack,
}

#[derive(Debug, Deserialize)]
struct Tx {
    #[serde(rename = "tx")]
    id: u32,
    #[serde(rename = "type")]
    ty: TxType,
    client: u16,
    amount: Option<Decimal>,
}

pub struct Account {
    client: u16,
    available: Decimal,
    held: Decimal,
    locked: bool,
}

impl Account {
    fn new(client: u16) -> Self {
        Self {
            client,
            available: Decimal::ZERO,
            held: Decimal::ZERO,
            locked: false,
        }
    }

    fn total(&self) -> Decimal {
        self.available + self.held
    }
}

// This struct is used to serialize the account summary to stdout
#[derive(Debug, Serialize)]
struct AccountSummary {
    client: u16,
    available: Decimal,
    held: Decimal,
    total: Decimal,
    locked: bool,
}

impl From<&Account> for AccountSummary {
    fn from(account: &Account) -> Self {
        Self {
            client: account.client,
            available: account.available.normalize(),
            held: account.held.normalize(),
            total: account.total().normalize(),
            locked: account.locked,
        }
    }
}

pub struct PaymentsEngine {
    input_file: String,
    // Stores deposit and withdrawal transactions that have not been reversed
    txs: HashMap<u32, Tx>,
    // Stores open disputes
    disputes: HashMap<u32, Tx>,
    accounts: HashMap<u16, Account>,
}

impl PaymentsEngine {
    pub fn new(input_file: String) -> Self {
        Self {
            input_file,
            txs: HashMap::new(),
            disputes: HashMap::new(),
            accounts: HashMap::new(),
        }
    }

    /// Process the transactions in the input file
    pub fn run(&mut self) {
        let file = std::fs::File::open(&self.input_file).unwrap();
        let mut reader = ReaderBuilder::new()
            .trim(Trim::All)
            .flexible(true)
            .from_reader(file);
        for res in reader.deserialize() {
            match res {
                Ok(tx) => {
                    if let Err(err) = self.process_tx(tx) {
                        eprintln!("Error: {}", err);
                    }
                }
                Err(err) => {
                    eprintln!(
                        "Failed to deserialize record: {}. Record will be skipped.",
                        err
                    );
                    continue;
                }
            }
        }
    }

    /// Serialize the accounts to stdout as CSV
    pub fn print_accounts<W: std::io::Write>(&self, writer: &mut W) {
        let mut writer = csv::Writer::from_writer(writer);
        for account in self.accounts.values() {
            writer
                .serialize(AccountSummary::from(account))
                .expect("Failed to serialize accounts to stdout");
        }
    }

    fn process_tx(&mut self, tx: Tx) -> Result<()> {
        let account = self
            .accounts
            .entry(tx.client)
            .or_insert(Account::new(tx.client));
        if account.locked {
            // Do not accept further transactions for locked accounts
            return Err(Error::AccountLocked(tx.client));
        }

        match tx.ty {
            TxType::Deposit | TxType::Withdrawal => {
                let amount = tx.amount.ok_or(Error::MissingTxAmount(tx.id))?;
                if amount <= Decimal::ZERO {
                    return Err(Error::TxInvalidAmount(tx.id));
                }

                match tx.ty {
                    TxType::Deposit => {
                        account.available += amount;
                    }
                    TxType::Withdrawal => {
                        if account.available < amount {
                            return Err(Error::NotEnoughFunds(tx.id));
                        }
                        account.available -= amount;
                    }
                    _ => unreachable!(),
                }
                self.txs.insert(tx.id, tx);
            }
            TxType::Dispute | TxType::Resolve | TxType::ChargeBack => {
                let original_tx = self.txs.get(&tx.id).ok_or(Error::TxDoesNotExist(tx.id))?;
                if tx.client != original_tx.client {
                    return Err(Error::ClientIdMismatch(tx.ty, tx.id));
                }

                match tx.ty {
                    TxType::Dispute => {
                        if !matches!(original_tx.ty, TxType::Deposit) {
                            // Only deposits can be disputed
                            return Err(Error::InvalidDispute(tx.id));
                        }

                        if self.disputes.contains_key(&tx.id) {
                            return Err(Error::TxAlreadyUnderDispute(tx.id));
                        }

                        let amount = original_tx
                            .amount
                            .expect("Deposit transaction has an amount");
                        account.available -= amount;
                        account.held += amount;
                        self.disputes.insert(tx.id, tx);
                    }
                    TxType::Resolve => {
                        // Cancellation of a dispute
                        self.disputes
                            .remove(&tx.id)
                            .ok_or(Error::TxNotUnderDispute(tx.id))?;
                        let amount = original_tx
                            .amount
                            .expect("Deposit transaction has an amount");
                        account.available += amount;
                        account.held -= amount;
                    }
                    TxType::ChargeBack => {
                        // Deposit reversal
                        let original_tx =
                            self.txs.get(&tx.id).ok_or(Error::TxDoesNotExist(tx.id))?;
                        self.disputes
                            .remove(&tx.id)
                            .ok_or(Error::TxNotUnderDispute(tx.id))?;
                        let amount = original_tx
                            .amount
                            .expect("Deposit transaction has an amount");
                        account.held -= amount;
                        account.locked = true;
                        self.txs.remove(&tx.id);
                    }
                    _ => unreachable!(),
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deposits_and_withdrawals() {
        let mut engine = PaymentsEngine::new("examples/deposits_and_withdrawals.csv".to_string());
        engine.run();
        assert_eq!(engine.accounts.len(), 2);
        assert_eq!(
            engine.accounts.get(&1).expect("Account exists").available,
            Decimal::ZERO
        );
        assert_eq!(
            engine.accounts.get(&2).unwrap().available,
            Decimal::new(4950, 2)
        );
    }

    #[test]
    fn test_failed_withdrawal() {
        let mut engine = PaymentsEngine::new("examples/failed_withdrawal.csv".to_string());
        engine.run();
        assert_eq!(
            engine.accounts.get(&1).expect("Account exists").available,
            Decimal::new(5000, 2)
        );
    }

    #[test]
    fn test_disputes() {
        let mut engine = PaymentsEngine::new("examples/disputes.csv".to_string());
        engine.run();

        // Client 1 dispute was resolved
        assert_eq!(
            engine.accounts.get(&1).expect("Account exists").available,
            Decimal::new(10000, 2)
        );
        assert_eq!(
            engine.accounts.get(&1).expect("Account exists").held,
            Decimal::ZERO
        );
        assert!(!engine.accounts.get(&1).expect("Account exists").locked);
        assert!(engine.disputes.get(&1).is_none());

        // Client 2 dispute is still open
        assert_eq!(
            engine.accounts.get(&2).expect("Account exists").available,
            Decimal::ZERO
        );
        assert_eq!(
            engine.accounts.get(&2).expect("Account exists").held,
            Decimal::new(10000, 2)
        );
        assert!(engine.disputes.get(&2).is_some());

        // Client 3 resolve ignored since no dispute opened
        assert_eq!(
            engine.accounts.get(&3).expect("Account exists").available,
            Decimal::new(10000, 2)
        );
        assert_eq!(
            engine.accounts.get(&3).expect("Account exists").held,
            Decimal::ZERO
        );
        assert!(engine.disputes.get(&3).is_none());
    }

    #[test]
    fn test_reversed_deposit() {
        let mut engine = PaymentsEngine::new("examples/reversed_deposit.csv".to_string());
        engine.run();

        // Deposit was reversed and the deposit following the chargeback was ignored
        assert!(engine.accounts.get(&1).expect("Account exists").locked);
        assert_eq!(
            engine.accounts.get(&1).expect("Account exists").available,
            Decimal::ZERO
        );
        assert_eq!(
            engine.accounts.get(&1).expect("Account exists").held,
            Decimal::ZERO
        );
        assert_eq!(engine.txs.len(), 0);
    }

    #[test]
    fn test_whitespace() {
        let mut engine = PaymentsEngine::new("examples/whitespace.csv".to_string());
        engine.run();
        assert_eq!(engine.accounts.len(), 1);
        assert_eq!(
            engine.accounts.get(&1).expect("Account exists").available,
            Decimal::new(9000, 2)
        );
    }

    #[test]
    fn test_print_accounts() {
        let mut engine = PaymentsEngine::new("examples/simple_deposit.csv".to_string());
        engine.run();

        let mut buf = Vec::new();
        engine.print_accounts(&mut buf);

        let expected = "client,available,held,total,locked\n1,100.1001,0,100.1001,false\n";
        assert_eq!(String::from_utf8(buf).unwrap(), expected);
    }
}
