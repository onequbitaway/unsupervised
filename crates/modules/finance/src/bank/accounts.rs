//! Money movements of bank accounts. Every change of an account balance
//! goes through this file, so that the deposit total and the low-water
//! marks stay right.

use unsup_kernel::{Dest, FaucetId, Holder, LedgerError, Money, Realm, ResidentId, SinkId};

use super::state::{Account, Bank};

impl Bank {
    /// Opens an account for a resident unless it has one. Returns true when
    /// the account is new. Money that the ledger already holds for the
    /// account counts as a deposit from now on.
    pub(crate) fn open(&mut self, realm: &Realm, resident: ResidentId, day: u64) -> bool {
        if self.accounts.contains_key(&resident.0) {
            return false;
        }
        self.accounts.insert(resident.0, Account { opened_day: day, ..Account::default() });
        self.deposits += Bank::balance(realm, resident);
        true
    }

    /// Records that the balance of an account went down.
    fn lowered(&mut self, realm: &Realm, resident: ResidentId) {
        let balance = Bank::balance(realm, resident);
        if let Some(account) = self.accounts.get_mut(&resident.0) {
            account.low_today = account.low_today.min(balance);
        }
    }

    /// Counts money that arrived in an account by wire from a zone.
    pub(crate) fn wired_in(&mut self, amount: i64) {
        self.deposits += amount;
    }

    /// Moves money from another holder of the global ledger into an
    /// account.
    pub(crate) fn pay_in(
        &mut self,
        realm: &Realm,
        from: Holder,
        resident: ResidentId,
        amount: i64,
        reason: &str,
    ) -> Result<(), LedgerError> {
        realm.ledger_mut().transfer_money(from, Bank::holder(resident), Money(amount), reason)?;
        self.deposits += amount;
        Ok(())
    }

    /// Moves money from an account to another holder of the global ledger
    /// that is not an account.
    pub(crate) fn pay_out(
        &mut self,
        realm: &Realm,
        resident: ResidentId,
        to: Holder,
        amount: i64,
        reason: &str,
    ) -> Result<(), LedgerError> {
        realm.ledger_mut().transfer_money(Bank::holder(resident), to, Money(amount), reason)?;
        self.deposits -= amount;
        self.lowered(realm, resident);
        Ok(())
    }

    /// Moves money from one account to another.
    pub(crate) fn move_between(
        &mut self,
        realm: &Realm,
        from: ResidentId,
        to: ResidentId,
        amount: i64,
        reason: &str,
    ) -> Result<(), LedgerError> {
        realm.ledger_mut().transfer_money(
            Bank::holder(from),
            Bank::holder(to),
            Money(amount),
            reason,
        )?;
        self.lowered(realm, from);
        Ok(())
    }

    /// Sends money from an account to the cash of a resident, wherever
    /// that resident is.
    pub(crate) fn wire_out(
        &mut self,
        realm: &Realm,
        resident: ResidentId,
        to: ResidentId,
        amount: i64,
        reason: &str,
    ) -> Result<(), LedgerError> {
        realm.wire_money(
            Bank::holder(resident),
            Dest::Resident(to),
            Holder::Resident(to),
            Money(amount),
            reason,
        )?;
        self.deposits -= amount;
        self.lowered(realm, resident);
        Ok(())
    }

    /// Closes an account. The caller has emptied it.
    pub(crate) fn close(&mut self, resident: ResidentId) {
        self.accounts.remove(&resident.0);
    }

    /// Creates money in an account through a faucet. Returns the cents
    /// that were created: `amount`, or 0 when the balance or the total of
    /// the faucet cannot hold it.
    pub(crate) fn mint_in(
        &mut self,
        realm: &Realm,
        faucet: FaucetId,
        resident: ResidentId,
        amount: i64,
    ) -> i64 {
        let holder = Bank::holder(resident);
        let before = realm.ledger().money(holder).0;
        realm.ledger_mut().mint(faucet, holder, Money(amount));
        let created = realm.ledger().money(holder).0 - before;
        self.deposits += created;
        created
    }

    /// Destroys money of an account through a sink.
    pub(crate) fn burn_out(
        &mut self,
        realm: &Realm,
        resident: ResidentId,
        sink: SinkId,
        amount: i64,
    ) -> Result<(), LedgerError> {
        realm.ledger_mut().burn(Bank::holder(resident), sink, Money(amount))?;
        self.deposits -= amount;
        self.lowered(realm, resident);
        Ok(())
    }
}
