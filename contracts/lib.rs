#![cfg_attr(not(feature = "std"), no_std)]

#[ink::contract]
mod open_payroll {
    use ink::prelude::string::String;
    use ink::prelude::vec::Vec;
    use ink::storage::traits::StorageLayout;
    use ink::storage::Mapping;

    // TODO: Review frame arbitrary precission numbers primitives
    type Multiplier = u128;

    #[derive(scale::Encode, scale::Decode, Eq, PartialEq, Debug, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo, StorageLayout))]
    pub struct Beneficiary {
        account_id: AccountId,
        multipliers: Vec<Multiplier>,
        unclaimed_payments: Balance,
        // TODO: Maybe this needs to be an option?
        last_claimed_period_block: BlockNumber,
    }

    #[ink(storage)]
    pub struct OpenPayroll {
        /// The accountId of the creator of the contract, who has 'priviliged' access to do administrative tasks
        owner: AccountId,
        /// Mapping with the accounts of the beneficiaries and the multiplier to apply to the base payment
        beneficiaries: Mapping<AccountId, Beneficiary>,
        // Vector of Accounts
        beneficiaries_accounts: Vec<AccountId>,
        /// We pay out every n blocks
        periodicity: u32,
        /// The amount of each base payment
        base_payment: Balance,
        /// The initial block number.
        initial_block: u32,
        /// The block number when the contract was paused
        paused_block_at: Option<u32>,
        /// The multipliers to apply to the base payment
        base_multipliers: Vec<String>,
    }

    #[derive(scale::Encode, scale::Decode, Eq, PartialEq, Debug, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        // The caller is not the owner of the contract
        NotOwner,
        // The contract is paused
        ContractIsPaused,
        // The params are invalid
        InvalidParams,
        // The account is not found
        AccountNotFound,
        // The contract does not have enough balance to pay
        NotEnoughBalanceInTreasury,
        // The transfer failed
        TransferFailed,
        // The beneficiary has no unclaimed payments
        NoUnclaimedPayments,
    }

    impl OpenPayroll {
        #[ink(constructor)]
        pub fn new(
            periodicity: u32,
            base_payment: Balance,
            base_multipliers: Vec<String>,
        ) -> Result<Self, Error> {
            let owner = Self::env().caller();
            // TODO: move this to a parameter

            if base_payment <= 0 || periodicity == 0 {
                return Err(Error::InvalidParams);
            }

            let beneficiaries = Mapping::default();
            Ok(Self {
                owner,
                beneficiaries,
                periodicity,
                base_payment,
                initial_block: Self::env().block_number(),
                paused_block_at: None,
                beneficiaries_accounts: Vec::default(),
                base_multipliers,
            })
        }

        // Ensure_owner ensures that the caller is the owner of the contract
        fn ensure_owner(&self) -> Result<(), Error> {
            let account = self.env().caller();
            // Only owners can call this function
            if self.owner != account {
                return Err(Error::NotOwner);
            }
            Ok(())
        }

        fn is_paused(&self) -> bool {
            self.paused_block_at.is_some()
        }

        // ensure_in_not_paused ensures that the contract is not paused
        fn ensure_in_not_paused(&self) -> Result<(), Error> {
            if self.is_paused() {
                return Err(Error::ContractIsPaused);
            }
            Ok(())
        }

        /// Add a new beneficiary or modify the multiplier of an existing one.
        #[ink(message)]
        pub fn add_or_update_beneficiary(
            &mut self,
            account_id: AccountId,
            multipliers: Vec<Multiplier>,
        ) -> Result<(), Error> {
            self.ensure_owner()?;

            // Check that the multipliers are valid and have the same length as the base_multipliers
            if multipliers.len() != self.base_multipliers.len() {
                return Err(Error::InvalidParams);
            }

            if let Some(beneficiary) = self.beneficiaries.get(&account_id) {
                // update the multiplier
                self.beneficiaries.insert(
                    account_id,
                    &Beneficiary {
                        account_id,
                        multipliers,
                        unclaimed_payments: beneficiary.unclaimed_payments,
                        last_claimed_period_block: beneficiary.last_claimed_period_block,
                    },
                );
            } else {
                // add a new beneficiary
                self.beneficiaries.insert(
                    account_id,
                    &Beneficiary {
                        account_id,
                        multipliers,
                        unclaimed_payments: 0,
                        last_claimed_period_block: 0,
                    },
                );
                self.beneficiaries_accounts.push(account_id);
            }
            Ok(())
        }

        /// Remove a beneficiary
        #[ink(message)]
        pub fn remove_beneficiary(&mut self, account_id: AccountId) -> Result<(), Error> {
            self.ensure_owner()?;
            if !self.beneficiaries.contains(&account_id) {
                return Err(Error::AccountNotFound);
            }
            self.beneficiaries.remove(&account_id);
            // remove the account from the vector
            if let Some(pos) = self
                .beneficiaries_accounts
                .iter()
                .position(|x| *x == account_id)
            {
                self.beneficiaries_accounts.remove(pos);
            }

            Ok(())
        }

        /// Update the base_payment
        #[ink(message)]
        pub fn update_base_payment(&mut self, base_payment: Balance) -> Result<(), Error> {
            self.ensure_owner()?;
            if base_payment == 0 {
                return Err(Error::InvalidParams);
            }

            // TODO: Sync unclaimed payments here
            self.base_payment = base_payment;

            Ok(())
        }

        /// Update the periodicity
        #[ink(message)]
        pub fn update_periodicity(&mut self, periodicity: u32) -> Result<(), Error> {
            self.ensure_owner()?;
            if periodicity == 0 {
                return Err(Error::InvalidParams);
            }

            // TODO: Sync unclaimed payments here
            self.periodicity = periodicity;

            Ok(())
        }

        /// Claim payment for a single account id
        #[ink(message)]
        pub fn claim_payment(&mut self) -> Result<(), Error> {
            self.ensure_in_not_paused()?;
            let account_id = self.env().caller();

            if !self.beneficiaries.contains(&account_id) {
                return Err(Error::AccountNotFound);
            }

            let beneficiary = self.beneficiaries.get(&account_id).unwrap();
            let current_block = self.env().block_number();

            // Calculates the number of blocks that have elapsed since the last payment
            let blocks_since_last_payment = current_block - beneficiary.last_claimed_period_block;

            // Calculates the number of payments that are due based on the elapsed blocks
            let unclaimed_periods: u128 = (blocks_since_last_payment / self.periodicity).into();
            if unclaimed_periods == 0 {
                return Err(Error::NoUnclaimedPayments);
            }

            //TODO Check if multipliers.length == base_multipliers.length
            // E.g (M1 + M2) * B / 100
            let final_multiplier: u128 = if beneficiary.multipliers.is_empty() {
                1
            } else {
                beneficiary.multipliers.iter().sum()
            };

            let payment_per_period: Balance = final_multiplier * self.base_payment / 100;
            let total_payment =
                payment_per_period * unclaimed_periods as u128 + beneficiary.unclaimed_payments;

            let treasury_balance = self.env().balance();
            if total_payment > treasury_balance {
                return Err(Error::NotEnoughBalanceInTreasury);
            }
            ink::env::debug_println!("total_payment: {}", total_payment);
            // Add the transfer checked if its failed
            if let Err(_) = self.env().transfer(account_id, total_payment) {
                return Err(Error::TransferFailed);
            }

            let claimed_period_block =
                current_block - ((current_block - self.initial_block) % self.periodicity);

            self.beneficiaries.insert(
                account_id,
                &Beneficiary {
                    account_id,
                    multipliers: beneficiary.multipliers,
                    unclaimed_payments: 0,
                    last_claimed_period_block: claimed_period_block,
                },
            );

            Ok(())
        }

        /// Calculate outstanding payments for the entire DAO -- this call can be expensive!!!
        #[ink(message)]
        pub fn calculate_outstanding_payments(&self) -> Result<Balance, Error> {
            todo!();
        }

        // TODO Add method to bulk add beneficiaries
        // #[ink(message)]
        // pub fn add_beneficiaries(&mut self, beneficiaries: Vec<AccountId, Multiplier>) {
        //     // let caller = self.env().caller();
        //     // assert_eq!(caller, self.owner, "Only the owner can add beneficiaries");
        //     // self.beneficiaries.push(account_id);
        //     // self.multipliers.insert(account_id, &multiplier);
        // }

        /// Pause the contract
        #[ink(message)]
        pub fn pause(&mut self) -> Result<(), Error> {
            self.ensure_owner()?;
            if self.is_paused() {
                return Ok(());
            }
            self.paused_block_at = Some(self.env().block_number());
            Ok(())
        }

        /// Resume the contract
        #[ink(message)]
        pub fn resume(&mut self) -> Result<(), Error> {
            self.ensure_owner()?;
            if !self.is_paused() {
                return Ok(());
            }
            self.paused_block_at = None;
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        // UTILITY FUNCTIONS TO MAKE TESTING EASIER
        fn create_contract(initial_balance: Balance) -> OpenPayroll {
            set_balance(contract_id(), initial_balance);
            OpenPayroll::new(
                2,
                1000,
                vec!["Seniority".to_string(), "Performance".to_string()],
            )
            .expect("Cannot create contract")
        }

        fn contract_id() -> AccountId {
            ink::env::test::callee::<ink::env::DefaultEnvironment>()
        }

        fn set_sender(sender: AccountId) {
            ink::env::test::set_caller::<ink::env::DefaultEnvironment>(sender);
        }

        fn default_accounts() -> ink::env::test::DefaultAccounts<ink::env::DefaultEnvironment> {
            ink::env::test::default_accounts::<ink::env::DefaultEnvironment>()
        }

        fn set_balance(account_id: AccountId, balance: Balance) {
            ink::env::test::set_account_balance::<ink::env::DefaultEnvironment>(account_id, balance)
        }

        fn advance_block() {
            ink::env::test::advance_block::<ink::env::DefaultEnvironment>();
        }

        fn get_current_block() -> u32 {
            ink::env::block_number::<ink::env::DefaultEnvironment>()
        }

        fn get_balance(account_id: AccountId) -> Balance {
            ink::env::test::get_account_balance::<ink::env::DefaultEnvironment>(account_id)
                .expect("Cannot get account balance")
        }

        /// We test if the default constructor does its job.
        #[ink::test]
        fn default_works() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            create_contract(100_000_000u128)
        }

        /// Add a new beneficiary and check that it is added
        #[ink::test]
        fn add_beneficiary() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract(100_000_000u128);
            contract
                .add_or_update_beneficiary(accounts.bob, vec![200, 100])
                .unwrap();
            assert_eq!(
                contract
                    .beneficiaries
                    .get(&accounts.bob)
                    .unwrap()
                    .multipliers,
                vec![200, 100]
            );
            contract
                .add_or_update_beneficiary(accounts.bob, vec![200, 50])
                .unwrap();
            assert_eq!(
                contract
                    .beneficiaries
                    .get(&accounts.bob)
                    .unwrap()
                    .multipliers,
                vec![200, 50]
            );
            // check if account was added to the vector
            assert_eq!(
                contract.beneficiaries_accounts.get(0).unwrap(),
                &accounts.bob
            );
        }

        /// Add a new beneficiary and fails because the sender is not the owner
        #[ink::test]
        fn add_beneficiary_without_access() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract(100_000_000u128);
            set_sender(accounts.bob);
            assert!(matches!(
                contract.add_or_update_beneficiary(accounts.bob, vec![100, 100]),
                Err(Error::NotOwner)
            ));
            // check if account was NOT added to the vector
            assert_eq!(contract.beneficiaries_accounts.len(), 0);
        }

        /// Add a new beneficiary and fails because the multiplies is 0
        #[ink::test]
        fn add_beneficiary_invalid_multiplier() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract(100_000_000u128);
            assert!(matches!(
                contract.add_or_update_beneficiary(accounts.bob, vec![]),
                Err(Error::InvalidParams)
            ));
        }

        /// Remove a beneficiary and check that it is removed
        #[ink::test]
        fn remove_beneficiary() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract(100_000_000u128);
            contract
                .add_or_update_beneficiary(accounts.bob, vec![100, 20])
                .unwrap();
            assert_eq!(contract.beneficiaries_accounts.len(), 1);
            assert_eq!(
                contract.beneficiaries_accounts.get(0).unwrap(),
                &accounts.bob
            );
            assert_eq!(
                contract
                    .beneficiaries
                    .get(&accounts.bob)
                    .unwrap()
                    .multipliers,
                vec![100, 20]
            );
            contract.remove_beneficiary(accounts.bob).unwrap();
            assert_eq!(contract.beneficiaries.contains(&accounts.bob), false);
            // check if account was removed from the vector
            assert_eq!(contract.beneficiaries_accounts.len(), 0);
        }

        /// Remove a beneficiary and fails because the sender is not the owner
        #[ink::test]
        fn remove_beneficiary_without_access() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract(100_000_000u128);
            contract
                .add_or_update_beneficiary(accounts.bob, vec![100, 20])
                .unwrap();
            set_sender(accounts.bob);
            assert!(matches!(
                contract.remove_beneficiary(accounts.bob),
                Err(Error::NotOwner)
            ));
            assert_eq!(contract.beneficiaries_accounts.len(), 1);
            assert_eq!(
                contract.beneficiaries_accounts.get(0).unwrap(),
                &accounts.bob
            );
        }

        /// Remove a beneficiary and fails because the beneficiary does not exist
        #[ink::test]
        fn remove_beneficiary_not_found() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract(100_000_000u128);
            assert!(matches!(
                contract.remove_beneficiary(accounts.bob),
                Err(Error::AccountNotFound)
            ));
        }

        /// Update the base payment and check that it is updated
        #[ink::test]
        fn update_base_payment() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract(100_000_000u128);
            contract.update_base_payment(200_000_000u128).unwrap();
            assert_eq!(contract.base_payment, 200_000_000u128);
        }

        /// Update the base payment but fails because the sender is not the owner
        #[ink::test]
        fn update_base_payment_without_access() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract(100_000_000u128);
            set_sender(accounts.bob);
            assert!(matches!(
                contract.update_base_payment(200_000_000u128),
                Err(Error::NotOwner)
            ));
        }

        /// Update the base payment but fails because the base payment is 0
        #[ink::test]
        fn update_base_payment_invalid_base_payment() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract(100_000_000u128);
            assert!(matches!(
                contract.update_base_payment(0u128),
                Err(Error::InvalidParams)
            ));
        }

        /// Update the periodicity and check that it is updated
        #[ink::test]
        fn update_periodicity() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract(100_000_000u128);
            contract.update_periodicity(100u32).unwrap();
            assert_eq!(contract.periodicity, 100u32);
        }

        /// Update the periodicity but fails because the sender is not the owner
        #[ink::test]
        fn update_periodicity_without_access() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract(100_000_000u128);
            set_sender(accounts.bob);
            assert!(matches!(
                contract.update_periodicity(100u32),
                Err(Error::NotOwner)
            ));
        }

        /// Update the periodicity but fails because the periodicity is 0
        #[ink::test]
        fn update_periodicity_invalid_periodicity() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract(100_000_000u128);
            assert!(matches!(
                contract.update_periodicity(0u32),
                Err(Error::InvalidParams)
            ));
        }

        /// Test pausing and unpausing the contract
        #[ink::test]
        fn pause_and_resume() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let starting_block = get_current_block();
            let mut contract = create_contract(100_000_000u128);
            contract.pause().unwrap();
            assert_eq!(contract.is_paused(), true);
            advance_block();
            contract.resume().unwrap();
            assert_eq!(contract.is_paused(), false);
            // check for the starting block to be the same
            assert_eq!(contract.initial_block, starting_block);
        }

        /// Test pausing and resuming without access
        #[ink::test]
        fn pause_and_resume_without_access() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract(100_000_000u128);
            set_sender(accounts.bob);
            assert!(matches!(contract.pause(), Err(Error::NotOwner)));
            assert!(matches!(contract.resume(), Err(Error::NotOwner)));
        }

        /// Test claiming a payment
        #[ink::test]
        fn claim_payment() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract(100_000_000u128);
            contract
                .add_or_update_beneficiary(accounts.bob, vec![100, 20])
                .unwrap();
            // advance 3 blocks so a payment will be claimable
            advance_block();
            advance_block();
            advance_block();
            let contract_balance_before_payment = get_balance(contract.owner);
            let bob_balance_before_payment = get_balance(accounts.bob);
            set_sender(accounts.bob);
            contract.claim_payment().unwrap();
            assert!(get_balance(contract.owner) < contract_balance_before_payment);
            assert!(get_balance(accounts.bob) > bob_balance_before_payment);
        }
    }
}
