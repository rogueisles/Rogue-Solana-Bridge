#![allow(unexpected_cfgs)]

use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    hash::hash,
    program::{invoke, invoke_signed},
    program_error::ProgramError,
    program_option::COption,
    program_pack::Pack,
    pubkey::Pubkey,
    rent::Rent,
    system_instruction, system_program,
    sysvar::Sysvar,
};
use spl_token::state::{Account as TokenAccount, AccountState, Mint};

#[cfg(not(feature = "no-entrypoint"))]
solana_program::entrypoint!(process_instruction);

pub const CONFIG_LEN: usize = 96;
pub const ESCROW_LEN: usize = 367;
pub const CONFIG_MAGIC: &[u8; 8] = b"RGNSPLC1";
pub const ESCROW_MAGIC: &[u8; 8] = b"RGNSPLE2";
pub const DECIMALS: u8 = 6;
pub const MAX_TTL: u64 = 604_800;
pub const ATA_PROGRAM: Pubkey =
    solana_program::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

#[repr(u32)]
#[derive(Clone, Copy, Debug)]
pub enum Error {
    Instruction = 1,
    Account = 2,
    Config = 3,
    Mint = 4,
    Token = 5,
    Pda = 6,
    Amount = 7,
    Deadline = 8,
    Settled = 9,
    Secret = 10,
    AlreadyInitialized = 11,
    Authority = 12,
}

impl From<Error> for ProgramError {
    fn from(value: Error) -> Self {
        ProgramError::Custom(value as u32)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub mint: Pubkey,
    pub domain: [u8; 32],
    pub min_ttl: u64,
    pub max_ttl: u64,
    pub max_amount: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Escrow {
    pub config: Pubkey,
    pub id: [u8; 32],
    pub hashlock: [u8; 32],
    pub claim_owner: Pubkey,
    pub claim_account: Pubkey,
    pub refund_account: Pubkey,
    pub funder: Pubkey,
    pub evm_recipient: [u8; 20],
    pub terms: [u8; 32],
    pub amount: u64,
    pub expiry: u64,
    pub state: u8,
    pub bump: u8,
    pub vault: Pubkey,
    pub direction: u8,
    pub secret: [u8; 32],
}

fn bytes<const N: usize>(data: &[u8], offset: usize) -> Result<[u8; N], ProgramError> {
    data.get(offset..offset + N)
        .ok_or(ProgramError::from(Error::Instruction))?
        .try_into()
        .map_err(|_| Error::Instruction.into())
}
fn key(data: &[u8], offset: usize) -> Result<Pubkey, ProgramError> {
    Ok(Pubkey::new_from_array(bytes(data, offset)?))
}
fn number(data: &[u8], offset: usize) -> Result<u64, ProgramError> {
    Ok(u64::from_le_bytes(bytes(data, offset)?))
}
fn put(data: &mut [u8], offset: usize, value: &[u8]) {
    data[offset..offset + value.len()].copy_from_slice(value);
}
fn require(condition: bool, error: Error) -> ProgramResult {
    if condition {
        Ok(())
    } else {
        Err(error.into())
    }
}

impl Config {
    pub fn decode(data: &[u8]) -> Result<Self, ProgramError> {
        require(
            data.len() == CONFIG_LEN && data.get(..8) == Some(CONFIG_MAGIC),
            Error::Config,
        )?;
        Ok(Self {
            mint: key(data, 8)?,
            domain: bytes(data, 40)?,
            min_ttl: number(data, 72)?,
            max_ttl: number(data, 80)?,
            max_amount: number(data, 88)?,
        })
    }
    fn encode(&self, data: &mut [u8]) {
        put(data, 0, CONFIG_MAGIC);
        put(data, 8, self.mint.as_ref());
        put(data, 40, &self.domain);
        put(data, 72, &self.min_ttl.to_le_bytes());
        put(data, 80, &self.max_ttl.to_le_bytes());
        put(data, 88, &self.max_amount.to_le_bytes());
    }
}

impl Escrow {
    pub fn decode(data: &[u8]) -> Result<Self, ProgramError> {
        require(
            data.len() == ESCROW_LEN && data.get(..8) == Some(ESCROW_MAGIC),
            Error::Account,
        )?;
        let value = Self {
            config: key(data, 8)?,
            id: bytes(data, 40)?,
            hashlock: bytes(data, 72)?,
            claim_owner: key(data, 104)?,
            claim_account: key(data, 136)?,
            refund_account: key(data, 168)?,
            funder: key(data, 200)?,
            evm_recipient: bytes(data, 232)?,
            terms: bytes(data, 252)?,
            amount: number(data, 284)?,
            expiry: number(data, 292)?,
            state: data[300],
            bump: data[301],
            vault: key(data, 302)?,
            direction: data[334],
            secret: bytes(data, 335)?,
        };
        require(
            match value.state {
                1 | 3 => value.secret == [0; 32],
                2 => hash(&value.secret).to_bytes() == value.hashlock,
                _ => false,
            },
            Error::Secret,
        )?;
        Ok(value)
    }
    fn encode(&self, data: &mut [u8]) {
        put(data, 0, ESCROW_MAGIC);
        put(data, 8, self.config.as_ref());
        put(data, 40, &self.id);
        put(data, 72, &self.hashlock);
        put(data, 104, self.claim_owner.as_ref());
        put(data, 136, self.claim_account.as_ref());
        put(data, 168, self.refund_account.as_ref());
        put(data, 200, self.funder.as_ref());
        put(data, 232, &self.evm_recipient);
        put(data, 252, &self.terms);
        put(data, 284, &self.amount.to_le_bytes());
        put(data, 292, &self.expiry.to_le_bytes());
        data[300] = self.state;
        data[301] = self.bump;
        put(data, 302, self.vault.as_ref());
        data[334] = self.direction;
        put(data, 335, &self.secret);
    }
}

fn check_program(account: &AccountInfo, expected: &Pubkey) -> ProgramResult {
    require(
        account.key == expected && account.executable,
        Error::Account,
    )
}

fn check_mint(account: &AccountInfo, expected: Option<&Pubkey>) -> Result<Mint, ProgramError> {
    require(
        account.owner == &spl_token::id() && !account.executable && account.data_len() == Mint::LEN,
        Error::Mint,
    )?;
    if let Some(mint) = expected {
        require(account.key == mint, Error::Mint)?;
    }
    let mint = Mint::unpack(&account.try_borrow_data()?)?;
    require(
        mint.is_initialized
            && mint.decimals == DECIMALS
            && mint.mint_authority == COption::None
            && mint.freeze_authority == COption::None,
        Error::Mint,
    )?;
    Ok(mint)
}

fn token(
    account: &AccountInfo,
    mint: &Pubkey,
    authority: &Pubkey,
) -> Result<TokenAccount, ProgramError> {
    require(
        account.owner == &spl_token::id()
            && !account.executable
            && account.data_len() == TokenAccount::LEN,
        Error::Token,
    )?;
    let value = TokenAccount::unpack(&account.try_borrow_data()?)?;
    require(
        value.state == AccountState::Initialized
            && value.mint == *mint
            && value.owner == *authority
            && value.is_native == COption::None,
        Error::Token,
    )?;
    Ok(value)
}

pub fn associated_address(authority: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[authority.as_ref(), spl_token::id().as_ref(), mint.as_ref()],
        &ATA_PROGRAM,
    )
    .0
}

fn check_ata(account: &AccountInfo, mint: &Pubkey, authority: &Pubkey) -> ProgramResult {
    require(
        account.key == &associated_address(authority, mint),
        Error::Token,
    )?;
    token(account, mint, authority)?;
    Ok(())
}

fn checked_config(account: &AccountInfo, program_id: &Pubkey) -> Result<Config, ProgramError> {
    require(
        account.owner == program_id && !account.executable,
        Error::Config,
    )?;
    Config::decode(&account.try_borrow_data()?)
}

fn now() -> Result<u64, ProgramError> {
    u64::try_from(Clock::get()?.unix_timestamp).map_err(|_| Error::Deadline.into())
}

fn allocate_pda<'a>(
    payer: &AccountInfo<'a>,
    target: &AccountInfo<'a>,
    system: &AccountInfo<'a>,
    owner: &Pubkey,
    size: usize,
    seeds: &[&[u8]],
) -> ProgramResult {
    require(
        target.is_writable
            && target.owner == &system_program::id()
            && target.data_is_empty()
            && !target.executable,
        Error::AlreadyInitialized,
    )?;
    let needed = Rent::get()?
        .minimum_balance(size)
        .saturating_sub(target.lamports());
    if needed > 0 {
        invoke(
            &system_instruction::transfer(payer.key, target.key, needed),
            &[payer.clone(), target.clone(), system.clone()],
        )?;
    }
    invoke_signed(
        &system_instruction::allocate(target.key, size as u64),
        &[target.clone(), system.clone()],
        &[seeds],
    )?;
    invoke_signed(
        &system_instruction::assign(target.key, owner),
        &[target.clone(), system.clone()],
        &[seeds],
    )
}

// InitConfig: tag0 | domain32 | minTTL u64 | maxTTL u64 | maxSPLBaseUnits u64.
// Accounts: new program-owned Config (signer,writable), approved classic SPL mint.
fn initialize(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    require(data.len() == 57 && accounts.len() == 2, Error::Instruction)?;
    let config = &accounts[0];
    let mint = &accounts[1];
    require(
        config.owner == program_id
            && config.is_signer
            && config.is_writable
            && config.data_len() == CONFIG_LEN
            && !config.executable,
        Error::Config,
    )?;
    require(
        config.try_borrow_data()?.iter().all(|value| *value == 0),
        Error::AlreadyInitialized,
    )?;
    require(
        Rent::get()?.is_exempt(config.lamports(), CONFIG_LEN),
        Error::Config,
    )?;
    check_mint(mint, None)?;
    let value = Config {
        mint: *mint.key,
        domain: bytes(data, 1)?,
        min_ttl: number(data, 33)?,
        max_ttl: number(data, 41)?,
        max_amount: number(data, 49)?,
    };
    require(
        value.domain != [0; 32]
            && value.min_ttl > 0
            && value.max_ttl >= value.min_ttl
            && value.max_ttl <= MAX_TTL
            && value.max_amount > 0,
        Error::Config,
    )?;
    value.encode(&mut config.try_borrow_mut_data()?);
    Ok(())
}

// Create: tag1 | id32 | SHA256(secret)32 | amount u64 | expiry u64 | claimOwner32
//         | evmRecipient20 | canonicalTermsHash32 | direction u8 (0=SPL source,1=SPL destination).
// Accounts: Config, Escrow PDA(w), Vault PDA(w), funder(s,w), source(w),
//           claim ATA, refund ATA, approved mint, classic Token executable, System executable.
fn create(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    require(
        data.len() == 166 && accounts.len() == 10,
        Error::Instruction,
    )?;
    let iter = &mut accounts.iter();
    let config = next_account_info(iter)?;
    let escrow = next_account_info(iter)?;
    let vault = next_account_info(iter)?;
    let funder = next_account_info(iter)?;
    let source = next_account_info(iter)?;
    let claim = next_account_info(iter)?;
    let refund = next_account_info(iter)?;
    let mint = next_account_info(iter)?;
    let token_program = next_account_info(iter)?;
    let system = next_account_info(iter)?;
    let cfg = checked_config(config, program_id)?;
    check_program(token_program, &spl_token::id())?;
    check_program(system, &system_program::id())?;
    check_mint(mint, Some(&cfg.mint))?;
    require(
        funder.is_signer
            && funder.is_writable
            && source.is_writable
            && escrow.key != vault.key
            && source.key != vault.key
            && claim.key != vault.key
            && refund.key != vault.key,
        Error::Authority,
    )?;
    let id: [u8; 32] = bytes(data, 1)?;
    let hashlock: [u8; 32] = bytes(data, 33)?;
    let amount = number(data, 65)?;
    let expiry = number(data, 73)?;
    let claim_owner = key(data, 81)?;
    let evm_recipient: [u8; 20] = bytes(data, 113)?;
    let terms: [u8; 32] = bytes(data, 133)?;
    let direction = data[165];
    require(
        id != [0; 32]
            && hashlock != [0; 32]
            && claim_owner != Pubkey::default()
            && evm_recipient != [0; 20]
            && terms != [0; 32]
            && direction <= 1,
        Error::Instruction,
    )?;
    require(amount > 0 && amount <= cfg.max_amount, Error::Amount)?;
    let time = now()?;
    require(
        expiry >= time.checked_add(cfg.min_ttl).ok_or(Error::Deadline)?
            && expiry <= time.checked_add(cfg.max_ttl).ok_or(Error::Deadline)?,
        Error::Deadline,
    )?;
    check_ata(claim, &cfg.mint, &claim_owner)?;
    check_ata(refund, &cfg.mint, funder.key)?;
    let source_state = token(source, &cfg.mint, funder.key)?;
    require(source_state.amount >= amount, Error::Amount)?;
    let (expected, bump) = Pubkey::find_program_address(
        &[b"swap", config.key.as_ref(), funder.key.as_ref(), &id],
        program_id,
    );
    require(escrow.key == &expected, Error::Pda)?;
    let (expected_vault, vault_bump) =
        Pubkey::find_program_address(&[b"vault", escrow.key.as_ref()], program_id);
    require(vault.key == &expected_vault, Error::Pda)?;
    let escrow_bump = [bump];
    let vault_bump_bytes = [vault_bump];
    let escrow_seeds = &[
        b"swap".as_ref(),
        config.key.as_ref(),
        funder.key.as_ref(),
        &id,
        &escrow_bump,
    ];
    let vault_seeds = &[b"vault".as_ref(), escrow.key.as_ref(), &vault_bump_bytes];
    allocate_pda(funder, escrow, system, program_id, ESCROW_LEN, escrow_seeds)?;
    allocate_pda(
        funder,
        vault,
        system,
        &spl_token::id(),
        TokenAccount::LEN,
        vault_seeds,
    )?;
    invoke(
        &spl_token::instruction::initialize_account3(
            &spl_token::id(),
            vault.key,
            mint.key,
            escrow.key,
        )?,
        &[vault.clone(), mint.clone(), token_program.clone()],
    )?;
    invoke(
        &spl_token::instruction::transfer_checked(
            &spl_token::id(),
            source.key,
            mint.key,
            vault.key,
            funder.key,
            &[],
            amount,
            DECIMALS,
        )?,
        &[
            source.clone(),
            mint.clone(),
            vault.clone(),
            funder.clone(),
            token_program.clone(),
        ],
    )?;
    let balance = token(vault, &cfg.mint, escrow.key)?;
    require(
        balance.amount == amount
            && balance.delegate == COption::None
            && balance.close_authority == COption::None,
        Error::Token,
    )?;
    Escrow {
        config: *config.key,
        id,
        hashlock,
        claim_owner,
        claim_account: *claim.key,
        refund_account: *refund.key,
        funder: *funder.key,
        evm_recipient,
        terms,
        amount,
        expiry,
        state: 1,
        bump,
        vault: *vault.key,
        direction,
        secret: [0; 32],
    }
    .encode(&mut escrow.try_borrow_mut_data()?);
    Ok(())
}

// Claim: tag2 | secret32. Refund: tag3. No authority signature is required.
// Accounts: Config, Escrow(w), Vault(w), fixed destination ATA(w), approved mint,
//           classic Token executable, fixed rent recipient(w).
fn settle(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
    is_claim: bool,
) -> ProgramResult {
    require(
        accounts.len() == 7 && data.len() == if is_claim { 33 } else { 1 },
        Error::Instruction,
    )?;
    let config = &accounts[0];
    let escrow = &accounts[1];
    let vault = &accounts[2];
    let destination = &accounts[3];
    let mint = &accounts[4];
    let token_program = &accounts[5];
    let rent_recipient = &accounts[6];
    let cfg = checked_config(config, program_id)?;
    require(
        escrow.owner == program_id
            && escrow.is_writable
            && vault.is_writable
            && destination.is_writable
            && !escrow.executable,
        Error::Account,
    )?;
    check_program(token_program, &spl_token::id())?;
    check_mint(mint, Some(&cfg.mint))?;
    let mut state = Escrow::decode(&escrow.try_borrow_data()?)?;
    require(
        state.config == *config.key && state.state == 1 && state.direction <= 1,
        Error::Settled,
    )?;
    let expected_rent_recipient = if state.direction == 0 {
        state.funder
    } else {
        state.claim_owner
    };
    require(
        rent_recipient.key == &expected_rent_recipient
            && rent_recipient.is_writable
            && rent_recipient.key != escrow.key
            && rent_recipient.key != vault.key
            && !rent_recipient.executable,
        Error::Authority,
    )?;
    let (expected, bump) = Pubkey::find_program_address(
        &[
            b"swap",
            config.key.as_ref(),
            state.funder.as_ref(),
            &state.id,
        ],
        program_id,
    );
    require(
        escrow.key == &expected
            && bump == state.bump
            && vault.key == &state.vault
            && vault.key
                == &Pubkey::find_program_address(&[b"vault", escrow.key.as_ref()], program_id).0,
        Error::Pda,
    )?;
    let balance = token(vault, &cfg.mint, escrow.key)?;
    require(
        balance.amount >= state.amount
            && balance.delegate == COption::None
            && balance.close_authority == COption::None,
        Error::Token,
    )?;
    let time = now()?;
    if is_claim {
        require(time < state.expiry, Error::Deadline)?;
        require(hash(&data[1..]).to_bytes() == state.hashlock, Error::Secret)?;
        require(destination.key == &state.claim_account, Error::Token)?;
        check_ata(destination, &cfg.mint, &state.claim_owner)?;
        state.secret = bytes(data, 1)?;
        state.state = 2;
    } else {
        require(time >= state.expiry, Error::Deadline)?;
        require(destination.key == &state.refund_account, Error::Token)?;
        check_ata(destination, &cfg.mint, &state.funder)?;
        state.state = 3;
    }
    state.encode(&mut escrow.try_borrow_mut_data()?);
    let bump_bytes = [state.bump];
    let seeds = &[
        b"swap".as_ref(),
        config.key.as_ref(),
        state.funder.as_ref(),
        &state.id,
        &bump_bytes,
    ];
    invoke_signed(
        &spl_token::instruction::transfer_checked(
            &spl_token::id(),
            vault.key,
            mint.key,
            destination.key,
            escrow.key,
            &[],
            state.amount,
            DECIMALS,
        )?,
        &[
            vault.clone(),
            mint.clone(),
            destination.clone(),
            escrow.clone(),
            token_program.clone(),
        ],
        &[seeds],
    )?;
    if balance.amount == state.amount {
        invoke_signed(
            &spl_token::instruction::close_account(
                &spl_token::id(),
                vault.key,
                rent_recipient.key,
                escrow.key,
                &[],
            )?,
            &[
                vault.clone(),
                rent_recipient.clone(),
                escrow.clone(),
                token_program.clone(),
            ],
            &[seeds],
        )?;
    }
    // Donated token dust keeps the nonempty vault open without blocking settlement.
    Ok(())
}

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    match data.first() {
        Some(0) => initialize(program_id, accounts, data),
        Some(1) => create(program_id, accounts, data),
        Some(2) => settle(program_id, accounts, data, true),
        Some(3) => settle(program_id, accounts, data, false),
        _ => Err(Error::Instruction.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn state_encoding_is_exact_and_keeps_terminal_state() {
        let cfg = Config {
            mint: Pubkey::new_unique(),
            domain: [7; 32],
            min_ttl: 5,
            max_ttl: MAX_TTL,
            max_amount: 1_000_000_000,
        };
        let mut data = vec![0; CONFIG_LEN];
        cfg.encode(&mut data);
        assert_eq!(Config::decode(&data).unwrap(), cfg);
        assert!(Config::decode(&data[..CONFIG_LEN - 1]).is_err());
        let value = Escrow {
            config: Pubkey::new_unique(),
            id: [1; 32],
            hashlock: [2; 32],
            claim_owner: Pubkey::new_unique(),
            claim_account: Pubkey::new_unique(),
            refund_account: Pubkey::new_unique(),
            funder: Pubkey::new_unique(),
            evm_recipient: [3; 20],
            terms: [4; 32],
            amount: u64::MAX,
            expiry: 1234,
            state: 3,
            bump: 251,
            vault: Pubkey::new_unique(),
            direction: 1,
            secret: [0; 32],
        };
        let mut data = vec![0; ESCROW_LEN];
        value.encode(&mut data);
        assert_eq!(Escrow::decode(&data).unwrap(), value);
        data[0] ^= 1;
        assert!(Escrow::decode(&data).is_err());
    }
}
