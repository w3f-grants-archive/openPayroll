#![cfg_attr(not(feature = "std"), no_std)]
mod errors;

#[ink::contract]
mod open_payroll {
    use crate::errors::Error;
    use ink::prelude::string::String;
    use ink::prelude::vec::Vec;
    use ink::storage::traits::StorageLayout;
    use ink::storage::Mapping;

    type Multiplier = u128;

    #[derive(scale::Encode, scale::Decode, Eq, PartialEq, Debug, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo, StorageLayout))]
    pub struct Beneficiary {
        account_id: AccountId,
        multipliers: Vec<Multiplier>,
        unclaimed_payments: Balance,
        last_claimed_period_block: BlockNumber,
    }

    #[derive(scale::Encode, scale::Decode, Eq, PartialEq, Debug, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo, StorageLayout))]
    pub struct InitialBeneficiary {
        account_id: AccountId,
        multipliers: Vec<Multiplier>,
    }

    #[derive(scale::Encode, scale::Decode, Eq, PartialEq, Debug, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo, StorageLayout))]
    pub struct ClaimsInPeriod {
        period: u32,
        total_claims: u32,
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
        /// Current claims in period
        claims_in_period: ClaimsInPeriod,
    }

    impl OpenPayroll {
        #[ink(constructor)]
        pub fn new(
            periodicity: u32,
            base_payment: Balance,
            base_multipliers: Vec<String>,
            initial_beneficiaries: Vec<InitialBeneficiary>,
        ) -> Result<Self, Error> {
            let initial_block_number = Self::env().block_number();
            let owner = Self::env().caller();
            // TODO: move this to a parameter

            if base_payment <= 0 || periodicity == 0 {
                return Err(Error::InvalidParams);
            }

            let mut beneficiaries = Mapping::default();
            let mut accounts = Vec::new();
            for beneficiary_data in initial_beneficiaries.iter() {
                // TODO: is this check ok ? or only if multipliers are bigger than base_multipliers.len()
                if beneficiary_data.multipliers.len() != base_multipliers.len() {
                    return Err(Error::InvalidMultipliersLength);
                }

                let beneficiary = Beneficiary {
                    account_id: beneficiary_data.account_id,
                    multipliers: beneficiary_data.multipliers.clone(),
                    unclaimed_payments: 0,
                    last_claimed_period_block: initial_block_number,
                };

                beneficiaries.insert(beneficiary_data.account_id, &beneficiary);
                accounts.push(beneficiary_data.account_id);
            }

            let claims_in_period = ClaimsInPeriod {
                period: 0,
                total_claims: 0,
            };

            Ok(Self {
                owner,
                beneficiaries,
                periodicity,
                base_payment,
                initial_block: initial_block_number,
                paused_block_at: None,
                beneficiaries_accounts: accounts,
                base_multipliers,
                claims_in_period,
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
            // TODO: this may be shorter than the base_multipliers
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

            //check if all payments are up to date
            //self.ensure_all_payments_uptodate()?;
            self.ensure_all_claimed_in_period()?;
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

            //check if all payments are up to date
            //self.ensure_all_payments_uptodate()?;
            self.ensure_all_claimed_in_period()?;
            self.periodicity = periodicity;

            Ok(())
        }

        /// Check if all payments up to date or storage unclaiumed_payments is up-to-date
        ///
        /// TODO: it would be better to return a vector of accounts that are not up to date ?
        #[ink(message)]
        pub fn ensure_all_payments_uptodate(&self) -> Result<(), Error> {
            let current_block = self.env().block_number();

            for account_id in self.beneficiaries_accounts.iter() {
                let beneficiary = self.beneficiaries.get(account_id).unwrap();
                let claimed_period_block =
                    current_block - ((current_block - self.initial_block) % self.periodicity);
                if claimed_period_block > beneficiary.last_claimed_period_block {
                    return Err(Error::PaymentsNotUpToDate);
                }
            }
            Ok(())
        }

        /// Get amount in storage without transferring the funds
        #[ink(message)]
        pub fn get_amount_to_claim(&mut self, account_id: AccountId) -> Result<Balance, Error> {
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

            Ok(total_payment)
        }

        /// Claim payment for a single account id
        #[ink(message)]
        pub fn claim_payment(
            &mut self,
            account_id: AccountId,
            amount: Balance,
        ) -> Result<(), Error> {
            self.ensure_in_not_paused()?;

            let current_block = self.env().block_number();

            let total_payment = Self::get_amount_to_claim(self, account_id)?;
            if amount > total_payment {
                return Err(Error::ClaimedAmountIsBiggerThanAvailable);
            }

            let beneficiary = self.beneficiaries.get(&account_id).expect(
                "This will never panic because we check it in the function get_amount_to_claim",
            );

            let treasury_balance = self.env().balance();
            if amount > treasury_balance {
                return Err(Error::NotEnoughBalanceInTreasury);
            }

            let claiming_period_block =
                current_block - ((current_block - self.initial_block) % self.periodicity);

            // If the beneficiary has not claimed anything in the current period
            if beneficiary.last_claimed_period_block != claiming_period_block {
                self.update_claims_in_period(claiming_period_block);
            }

            self.beneficiaries.insert(
                account_id,
                &Beneficiary {
                    account_id,
                    multipliers: beneficiary.multipliers,
                    unclaimed_payments: total_payment - amount,
                    last_claimed_period_block: claiming_period_block,
                },
            );

            // Transfer the amount to the beneficiary if amount > 0
            if amount > 0 {
                if let Err(_) = self.env().transfer(account_id, amount) {
                    return Err(Error::TransferFailed);
                }
            }

            Ok(())
        }

        pub fn update_claims_in_period(&mut self, claiming_period_block: BlockNumber) {
            if claiming_period_block == self.claims_in_period.period {
                // Updates current claims in period
                self.claims_in_period.total_claims += 1;
            } else {
                // Reset the claims in period
                self.claims_in_period.period = claiming_period_block;
                self.claims_in_period.total_claims = 1;
            }
        }

        fn ensure_all_claimed_in_period(&mut self) -> Result<(), Error> {
            let current_block = self.env().block_number();
            let claiming_period_block =
                current_block - ((current_block - self.initial_block) % self.periodicity);

            let claims_in_period = self.claims_in_period.clone();

            if (claiming_period_block == claims_in_period.period
                && claims_in_period.total_claims == self.beneficiaries_accounts.len() as u32)
                || claiming_period_block == 0
            // initial period in intial block noone can claim
            {
                return Ok(());
            }

            return Err(Error::NotAllClaimedInPeriod);
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

        /// Get beneficiary only read
        #[ink(message)]
        pub fn get_beneficiary(&mut self, account_id: AccountId) -> Result<Beneficiary, Error> {
            if !self.beneficiaries.contains(&account_id) {
                return Err(Error::AccountNotFound);
            }
            let beneficiary = self.beneficiaries.get(&account_id).unwrap();
            Ok(beneficiary)
        }
    }

    #[cfg(test)]
    mod tests {
        use ink::env::{test::DefaultAccounts, DefaultEnvironment};

        use super::*;

        // UTILITY FUNCTIONS TO MAKE TESTING EASIER
        fn create_contract(
            initial_balance: Balance,
            accounts: &DefaultAccounts<DefaultEnvironment>,
        ) -> OpenPayroll {
            set_balance(contract_id(), initial_balance);
            let beneficiary_bob = InitialBeneficiary {
                account_id: accounts.bob,
                multipliers: vec![100, 3],
            };
            let beneficiary_charlie = InitialBeneficiary {
                account_id: accounts.charlie,
                multipliers: vec![100, 3],
            };
            OpenPayroll::new(
                2,
                1000,
                vec!["Seniority".to_string(), "Performance".to_string()],
                vec![beneficiary_bob, beneficiary_charlie],
            )
            .expect("Cannot create contract")
        }

        fn create_contract_with_no_beneficiaries(initial_balance: Balance) -> OpenPayroll {
            set_balance(contract_id(), initial_balance);
            OpenPayroll::new(
                2,
                1000,
                vec!["Seniority".to_string(), "Performance".to_string()],
                vec![],
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

        fn advance_n_blocks(n: u32) {
            for _ in 0..n {
                ink::env::test::advance_block::<ink::env::DefaultEnvironment>();
            }
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
            create_contract(100_000_000u128, &accounts)
        }

        #[ink::test]
        fn create_contract_with_invalid_amount_of_multipliers() {
            let accounts = default_accounts();
            let beneficiary_bob = InitialBeneficiary {
                account_id: accounts.bob,
                multipliers: vec![100, 3],
            };
            let beneficiary_charlie = InitialBeneficiary {
                account_id: accounts.charlie,
                multipliers: vec![100],
            };
            let res = OpenPayroll::new(
                2,
                1000,
                vec!["Seniority".to_string(), "Performance".to_string()],
                vec![beneficiary_bob, beneficiary_charlie],
            );

            assert!(matches!(res, Err(Error::InvalidMultipliersLength)));

            let beneficiary_bob = InitialBeneficiary {
                account_id: accounts.bob,
                multipliers: vec![100],
            };
            let beneficiary_charlie = InitialBeneficiary {
                account_id: accounts.charlie,
                multipliers: vec![100],
            };
            let res = OpenPayroll::new(
                2,
                1000,
                vec!["Seniority".to_string(), "Performance".to_string()],
                vec![beneficiary_bob, beneficiary_charlie],
            );

            assert!(matches!(res, Err(Error::InvalidMultipliersLength)));

            let beneficiary_bob = InitialBeneficiary {
                account_id: accounts.bob,
                multipliers: vec![],
            };
            let beneficiary_charlie = InitialBeneficiary {
                account_id: accounts.charlie,
                multipliers: vec![],
            };
            let res = OpenPayroll::new(
                2,
                1000,
                vec!["Seniority".to_string(), "Performance".to_string()],
                vec![beneficiary_bob, beneficiary_charlie],
            );

            assert!(matches!(res, Err(Error::InvalidMultipliersLength)));

            let beneficiary_bob = InitialBeneficiary {
                account_id: accounts.bob,
                multipliers: vec![10, 3, 3],
            };
            let beneficiary_charlie = InitialBeneficiary {
                account_id: accounts.charlie,
                multipliers: vec![10, 3],
            };
            let res = OpenPayroll::new(
                2,
                1000,
                vec![
                    "Seniority".to_string(),
                    "Performance".to_string(),
                    "Years_at_company".to_string(),
                ],
                vec![beneficiary_bob, beneficiary_charlie],
            );

            assert!(matches!(res, Err(Error::InvalidMultipliersLength)));
        }

        /// Add a new beneficiary and check that it is added
        #[ink::test]
        fn add_beneficiary() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract_with_no_beneficiaries(100_000_000u128);
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
            let mut contract = create_contract_with_no_beneficiaries(100_000_000u128);
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
            let mut contract = create_contract_with_no_beneficiaries(100_000_000u128);
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
            let mut contract = create_contract_with_no_beneficiaries(100_000_000u128);
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
            let mut contract = create_contract_with_no_beneficiaries(100_000_000u128);
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
            let mut contract = create_contract_with_no_beneficiaries(100_000_000u128);
            assert!(matches!(
                contract.remove_beneficiary(accounts.bob),
                Err(Error::AccountNotFound)
            ));
        }

        /// Update the base payment and check that it is updated
        #[ink::test]
        fn update_base_payment_in_initial_block() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract(100_000_000u128, &accounts);
            contract.update_base_payment(200_000_000u128).unwrap();
            assert_eq!(contract.base_payment, 200_000_000u128);
        }

        /// Update the base payment and check that it is updated
        #[ink::test]
        fn update_base_payment() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract(100_000_000u128, &accounts);

            advance_n_blocks(1);

            contract.update_base_payment(200_000_000u128).unwrap();
            assert_eq!(contract.base_payment, 200_000_000u128);
        }

        #[ink::test]
        fn update_base_payment_error() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract(100_000_000u128, &accounts);

            advance_n_blocks(3);

            assert!(matches!(
                contract.update_base_payment(200_000_000u128),
                Err(Error::NotAllClaimedInPeriod)
            ));
        }

        /// Update the base payment but fails because the sender is not the owner
        #[ink::test]
        fn update_base_payment_without_access() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract(100_000_000u128, &accounts);
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
            let mut contract = create_contract(100_000_000u128, &accounts);
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
            let mut contract = create_contract(100_000_000u128, &accounts);
            contract.update_periodicity(100u32).unwrap();
            assert_eq!(contract.periodicity, 100u32);
        }

        /// Update the periodicity but fails because the sender is not the owner
        #[ink::test]
        fn update_periodicity_without_access() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract(100_000_000u128, &accounts);
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
            let mut contract = create_contract(100_000_000u128, &accounts);
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
            let mut contract = create_contract(100_000_000u128, &accounts);
            contract.pause().unwrap();
            assert_eq!(contract.is_paused(), true);
            advance_n_blocks(1);
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
            let mut contract = create_contract(100_000_000u128, &accounts);
            set_sender(accounts.bob);
            assert!(matches!(contract.pause(), Err(Error::NotOwner)));
            assert!(matches!(contract.resume(), Err(Error::NotOwner)));
        }

        /// Test claiming a payment
        #[ink::test]
        fn claim_payment() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract(100_000_000u128, &accounts);
            contract
                .add_or_update_beneficiary(accounts.bob, vec![100, 20])
                .unwrap();
            // advance 3 blocks so a payment will be claimable
            advance_n_blocks(3);

            let contract_balance_before_payment = get_balance(contract.owner);
            let bob_balance_before_payment = get_balance(accounts.bob);
            set_sender(accounts.bob);

            let amount_to_claim = contract.get_amount_to_claim(accounts.bob).unwrap();
            contract
                .claim_payment(accounts.bob, amount_to_claim)
                .unwrap();
            assert!(get_balance(contract.owner) < contract_balance_before_payment);
            assert!(get_balance(accounts.bob) > bob_balance_before_payment);
        }

        /// Test claiming a payment
        #[ink::test]
        fn claim_parcial_payment() {
            let accounts = default_accounts();
            let total_amount = 100_000_000u128;
            let total_not_claimed = 10;
            set_sender(accounts.alice);
            let mut contract = create_contract(total_amount, &accounts);
            contract
                .add_or_update_beneficiary(accounts.bob, vec![100, 20])
                .unwrap();

            // advance 3 blocks so a payment will be claimable
            advance_n_blocks(3);

            let bob_balance_before_payment = get_balance(accounts.bob);
            set_sender(accounts.bob);

            let amount_to_claim = contract.get_amount_to_claim(accounts.bob).unwrap();
            contract
                .claim_payment(accounts.bob, amount_to_claim - total_not_claimed)
                .unwrap();
            assert!(
                get_balance(contract.owner) == total_amount - amount_to_claim + total_not_claimed
            );
            assert!(
                get_balance(accounts.bob)
                    == bob_balance_before_payment + amount_to_claim - total_not_claimed
            );
            assert!(
                contract
                    .beneficiaries
                    .get(accounts.bob)
                    .unwrap()
                    .unclaimed_payments
                    == total_not_claimed
            );
        }

        /// Test claiming a payment
        #[ink::test]
        fn claim_more_payment() {
            let accounts = default_accounts();
            let total_amount = 100_000_000u128;
            set_sender(accounts.alice);
            let mut contract = create_contract(total_amount, &accounts);
            contract
                .add_or_update_beneficiary(accounts.bob, vec![100, 20])
                .unwrap();

            // advance 3 blocks so a payment will be claimable
            advance_n_blocks(3);

            let bob_balance_before_payment = get_balance(accounts.bob);
            set_sender(accounts.bob);

            let amount_to_claim = contract.get_amount_to_claim(accounts.bob).unwrap();
            let res = contract.claim_payment(accounts.bob, amount_to_claim + 1);

            assert!(matches!(
                res,
                Err(Error::ClaimedAmountIsBiggerThanAvailable)
            ));
            assert!(get_balance(contract.owner) == total_amount);
            assert!(get_balance(accounts.bob) == bob_balance_before_payment);
        }

        #[ink::test]
        fn update_periodicity_without_all_payments_updated() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract(100_000_000u128, &accounts);
            contract
                .add_or_update_beneficiary(accounts.bob, vec![100, 20])
                .unwrap();

            // advance 3 blocks so a payment will be claimable
            advance_n_blocks(3);

            let res = contract.update_periodicity(10u32);
            assert!(matches!(res, Err(Error::NotAllClaimedInPeriod)));
        }

        #[ink::test]
        fn update_periodicity_with_all_payments_updated() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract_with_no_beneficiaries(100_000_000u128);
            contract
                .add_or_update_beneficiary(accounts.bob, vec![100, 20])
                .unwrap();
            // advance 3 blocks so a payment will be claimable
            advance_n_blocks(3);

            // When you claim a payment with 0 amount, it will calculate the amount to claim an set it to unclaim payments.
            contract.claim_payment(accounts.bob, 0).unwrap();

            let res = contract.update_periodicity(10u32);

            assert!(matches!(res, Ok(())));
        }

        #[ink::test]
        fn update_periodicity_with_all_payments_claimed() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract_with_no_beneficiaries(100_000_000u128);
            contract
                .add_or_update_beneficiary(accounts.bob, vec![100, 20])
                .unwrap();
            // advance 3 blocks so a payment will be claimable
            advance_n_blocks(3);

            set_sender(accounts.bob);

            let amount_to_claim = contract.get_amount_to_claim(accounts.bob).unwrap();
            contract
                .claim_payment(accounts.bob, amount_to_claim)
                .unwrap();

            set_sender(accounts.alice);
            let res = contract.update_periodicity(10u32);

            assert_eq!(res, Ok(()));
        }

        #[ink::test]
        fn update_base_payment_without_all_payments_updated() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract_with_no_beneficiaries(100_000_000u128);
            contract
                .add_or_update_beneficiary(accounts.bob, vec![100, 20])
                .unwrap();
            // advance 3 blocks so a payment will be claimable
            advance_n_blocks(3);

            let res = contract.update_base_payment(900);

            assert!(matches!(res, Err(Error::NotAllClaimedInPeriod)));
        }

        #[ink::test]
        fn update_base_payment_with_all_payments_claimed() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract_with_no_beneficiaries(100_000_000u128);
            contract
                .add_or_update_beneficiary(accounts.bob, vec![100, 20])
                .unwrap();
            // advance 3 blocks so a payment will be claimable
            advance_n_blocks(3);

            set_sender(accounts.bob);
            let amount_to_claim = contract.get_amount_to_claim(accounts.bob).unwrap();
            contract
                .claim_payment(accounts.bob, amount_to_claim)
                .unwrap();

            set_sender(accounts.alice);
            let res = contract.update_base_payment(900);

            assert_eq!(res, Ok(()));
        }

        #[ink::test]
        fn create_contract_with_beneficiaries_ok() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let contract = create_contract(100_000_000u128, &accounts);

            assert_eq!(contract.beneficiaries_accounts.len(), 2);
            assert!(contract.beneficiaries.contains(accounts.bob));
            assert!(contract.beneficiaries.contains(accounts.charlie));
        }

        #[ink::test]
        fn update_benefiaries_created_in_create_contract() {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            let mut contract = create_contract(100_000_000u128, &accounts);

            contract
                .add_or_update_beneficiary(accounts.bob, vec![100, 20])
                .unwrap();

            //check if multipliers are ok
            assert_eq!(
                contract
                    .beneficiaries
                    .get(accounts.bob)
                    .unwrap()
                    .multipliers,
                vec![100, 20]
            );
            assert_eq!(
                contract
                    .beneficiaries
                    .get(accounts.charlie)
                    .unwrap()
                    .multipliers,
                vec![100, 3]
            );
        }

        // TODO: delete a multiplier
    }
}
