#![allow(unexpected_cfgs)]

use rgn_spl_escrow::{associated_address, Config, Escrow, ESCROW_LEN};
use solana_program::{
    account_info::AccountInfo,
    clock::Clock,
    entrypoint::ProgramResult,
    instruction::{AccountMeta, Instruction},
    program::{invoke, invoke_signed},
    program_error::ProgramError,
    program_option::COption,
    program_pack::Pack,
    pubkey::Pubkey,
    rent::Rent,
    system_instruction, system_program,
    sysvar::{
        instructions::{self, load_current_index_checked, load_instruction_at_checked},
        Sysvar,
    },
};
use spl_token::state::{Account as TokenAccount, AccountState, Mint};

#[cfg(not(feature = "no-entrypoint"))]
solana_program::entrypoint!(process_instruction);

pub const HEADER_LEN: usize = 528;
pub const ENTRY_LEN: usize = 56;
pub const CAPACITY: usize = 128;
pub const STATE_LEN: usize = HEADER_LEN + ENTRY_LEN * CAPACITY;
pub const PROOF_LEN: usize = 116;
pub const MAGIC: &[u8; 8] = b"RGNSPLQ2";
pub const PROOF_MAGIC: &[u8; 8] = b"RGNPRFQ1";
pub const WINDOW: u64 = 3600;
pub const HOUR_LIMIT: u64 = 10_000_000_000_000;
pub const MAX_PENDING: u16 = 8;
pub const APPROVAL_TTL: u64 = 300;
pub const ADMIN_DELAY: u64 = 86_400;
pub const NAMESPACE: u8 = 0x80;

#[repr(u32)]
#[derive(Clone, Copy, Debug)]
pub enum Error {
    Instruction = 101,
    Account = 102,
    Authority = 103,
    Config = 104,
    Amount = 105,
    Capacity = 106,
    Budget = 107,
    Paused = 108,
    NotSettled = 109,
    AlreadySettled = 110,
    Deadline = 111,
    Epoch = 112,
    Replay = 113,
    Rotation = 114,
    Governance = 115,
    Nonce = 116,
}
impl From<Error> for ProgramError {
    fn from(value: Error) -> Self {
        ProgramError::Custom(value as u32)
    }
}
fn require(condition: bool, error: Error) -> ProgramResult {
    if condition {
        Ok(())
    } else {
        Err(error.into())
    }
}
fn bytes<const N: usize>(data: &[u8], start: usize) -> Result<[u8; N], ProgramError> {
    data.get(start..start + N)
        .ok_or(ProgramError::from(Error::Instruction))?
        .try_into()
        .map_err(|_| Error::Instruction.into())
}
fn key(data: &[u8], start: usize) -> Result<Pubkey, ProgramError> {
    Ok(Pubkey::new_from_array(bytes(data, start)?))
}
fn num(data: &[u8], start: usize) -> Result<u64, ProgramError> {
    Ok(u64::from_le_bytes(bytes(data, start)?))
}
fn small(data: &[u8], start: usize) -> Result<u16, ProgramError> {
    Ok(u16::from_le_bytes(bytes(data, start)?))
}
fn epoch(data: &[u8], start: usize) -> Result<u32, ProgramError> {
    Ok(u32::from_le_bytes(bytes(data, start)?))
}
fn put(data: &mut [u8], start: usize, value: &[u8]) {
    data[start..start + value.len()].copy_from_slice(value);
}
fn now() -> Result<u64, ProgramError> {
    u64::try_from(Clock::get()?.unix_timestamp).map_err(|_| Error::Deadline.into())
}

#[derive(Clone)]
pub struct Reserve {
    pub core_program: Pubkey,
    pub core_config: Pubkey,
    pub mint: Pubkey,
    pub administrator: Pubkey,
    pub funding: Pubkey,
    pub route: [u8; 32],
    pub approvers: [Pubkey; 4],
    pub epoch: u32,
    pub max_swap: u64,
    pub max_gross: u64,
    pub pending: u16,
    pub paused: bool,
    pub bump: u8,
    pub governance_after: u64,
    pub governance_limit: u64,
    pub governance_epoch: u32,
    pub governance_kind: u8,
    pub governance_nonce: u64,
    pub pending_nonce: u64,
    pub next_approvers: [Pubkey; 4],
}
impl Reserve {
    pub fn decode(data: &[u8]) -> Result<Self, ProgramError> {
        require(
            data.len() == STATE_LEN && data.get(..8) == Some(MAGIC),
            Error::Account,
        )?;
        require(
            num(data, 332)? > 0
                && num(data, 340)? > 0
                && num(data, 348)? == WINDOW
                && small(data, 356)? == MAX_PENDING,
            Error::Config,
        )?;
        Ok(Self {
            core_program: key(data, 8)?,
            core_config: key(data, 40)?,
            mint: key(data, 72)?,
            administrator: key(data, 104)?,
            funding: key(data, 136)?,
            route: bytes(data, 168)?,
            approvers: [
                key(data, 200)?,
                key(data, 232)?,
                key(data, 264)?,
                key(data, 296)?,
            ],
            epoch: epoch(data, 328)?,
            max_swap: num(data, 332)?,
            max_gross: num(data, 340)?,
            pending: small(data, 358)?,
            paused: data[360] != 0,
            bump: data[361],
            governance_after: num(data, 362)?,
            governance_limit: num(data, 370)?,
            governance_epoch: epoch(data, 378)?,
            governance_kind: data[510],
            governance_nonce: num(data, 512)?,
            pending_nonce: num(data, 520)?,
            next_approvers: [
                key(data, 382)?,
                key(data, 414)?,
                key(data, 446)?,
                key(data, 478)?,
            ],
        })
    }
    fn encode(&self, data: &mut [u8]) {
        put(data, 0, MAGIC);
        put(data, 8, self.core_program.as_ref());
        put(data, 40, self.core_config.as_ref());
        put(data, 72, self.mint.as_ref());
        put(data, 104, self.administrator.as_ref());
        put(data, 136, self.funding.as_ref());
        put(data, 168, &self.route);
        for i in 0..4 {
            put(data, 200 + i * 32, self.approvers[i].as_ref());
            put(data, 382 + i * 32, self.next_approvers[i].as_ref());
        }
        put(data, 328, &self.epoch.to_le_bytes());
        put(data, 332, &self.max_swap.to_le_bytes());
        put(data, 340, &self.max_gross.to_le_bytes());
        put(data, 348, &WINDOW.to_le_bytes());
        put(data, 356, &MAX_PENDING.to_le_bytes());
        put(data, 358, &self.pending.to_le_bytes());
        data[360] = u8::from(self.paused);
        data[361] = self.bump;
        put(data, 362, &self.governance_after.to_le_bytes());
        put(data, 370, &self.governance_limit.to_le_bytes());
        put(data, 378, &self.governance_epoch.to_le_bytes());
        data[510] = self.governance_kind;
        put(data, 512, &self.governance_nonce.to_le_bytes());
        put(data, 520, &self.pending_nonce.to_le_bytes());
    }
    fn clear_governance(&mut self) {
        self.governance_after = 0;
        self.governance_limit = 0;
        self.governance_epoch = 0;
        self.governance_kind = 0;
        self.pending_nonce = 0;
        self.next_approvers = [Pubkey::default(); 4];
    }
}
fn checked(account: &AccountInfo, program: &Pubkey) -> Result<Reserve, ProgramError> {
    require(
        account.owner == program && !account.executable && account.is_writable,
        Error::Account,
    )?;
    let reserve = Reserve::decode(&account.try_borrow_data()?)?;
    require(reserve.core_program == *program, Error::Config)?;
    Ok(reserve)
}
fn core_config(
    config: &AccountInfo,
    mint: &AccountInfo,
    program: &AccountInfo,
) -> Result<Config, ProgramError> {
    require(
        program.executable && config.owner == program.key && !config.executable,
        Error::Config,
    )?;
    let cfg = Config::decode(&config.try_borrow_data()?)?;
    require(
        mint.key == &cfg.mint
            && mint.owner == &spl_token::id()
            && mint.data_len() == Mint::LEN
            && !mint.executable,
        Error::Config,
    )?;
    let m = Mint::unpack(&mint.try_borrow_data()?)?;
    require(
        m.is_initialized
            && m.decimals == 6
            && m.mint_authority == COption::None
            && m.freeze_authority == COption::None,
        Error::Config,
    )?;
    Ok(cfg)
}
fn valid_keys(keys: &[Pubkey; 4], admin: &Pubkey) -> ProgramResult {
    for i in 0..4 {
        require(
            keys[i] != Pubkey::default() && &keys[i] != admin,
            Error::Authority,
        )?;
        for j in 0..i {
            require(keys[i] != keys[j], Error::Authority)?;
        }
    }
    Ok(())
}
fn quorum(reserve: &Reserve, signers: &[AccountInfo]) -> ProgramResult {
    require(signers.len() == 3, Error::Authority)?;
    let mut seen = [false; 4];
    for signer in signers {
        require(signer.is_signer, Error::Authority)?;
        let index = reserve
            .approvers
            .iter()
            .position(|key| key == signer.key)
            .ok_or(Error::Authority)?;
        require(!seen[index], Error::Authority)?;
        seen[index] = true;
    }
    Ok(())
}
fn single_instruction(
    program: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
    sysvar: &AccountInfo,
) -> ProgramResult {
    require(
        sysvar.key == &instructions::id() && load_current_index_checked(sysvar)? == 0,
        Error::Instruction,
    )?;
    let instruction = load_instruction_at_checked(0, sysvar)?;
    require(
        instruction.program_id == *program
            && instruction.data.first() == Some(&NAMESPACE)
            && instruction.data.get(1..) == Some(data)
            && instruction.accounts.len() == accounts.len(),
        Error::Instruction,
    )?;
    for (meta, account) in instruction.accounts.iter().zip(accounts) {
        require(meta.pubkey == *account.key, Error::Account)?;
    }
    require(
        load_instruction_at_checked(1, sysvar).is_err(),
        Error::Instruction,
    )
}
fn allocate_proof<'a>(
    payer: &AccountInfo<'a>,
    target: &AccountInfo<'a>,
    system: &AccountInfo<'a>,
    program: &Pubkey,
    seeds: &[&[u8]],
) -> ProgramResult {
    require(
        target.is_writable
            && target.owner == &system_program::id()
            && target.data_is_empty()
            && !target.executable,
        Error::Replay,
    )?;
    let needed = Rent::get()?
        .minimum_balance(PROOF_LEN)
        .saturating_sub(target.lamports());
    if needed > 0 {
        invoke(
            &system_instruction::transfer(payer.key, target.key, needed),
            &[payer.clone(), target.clone(), system.clone()],
        )?;
    }
    invoke_signed(
        &system_instruction::allocate(target.key, PROOF_LEN as u64),
        &[target.clone(), system.clone()],
        &[seeds],
    )?;
    invoke_signed(
        &system_instruction::assign(target.key, program),
        &[target.clone(), system.clone()],
        &[seeds],
    )
}

fn init(program: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    require(accounts.len() == 5 && data.len() == 161, Error::Instruction)?;
    require(accounts[4].key == program, Error::Config)?;
    let state = &accounts[0];
    let admin = &accounts[1];
    require(
        state.owner == program
            && state.is_signer
            && state.is_writable
            && state.data_len() == STATE_LEN
            && !state.executable,
        Error::Account,
    )?;
    require(
        state.try_borrow_data()?.iter().all(|b| *b == 0)
            && Rent::get()?.is_exempt(state.lamports(), STATE_LEN),
        Error::Account,
    )?;
    require(
        admin.is_signer && *admin.key != Pubkey::default(),
        Error::Authority,
    )?;
    let cfg = core_config(&accounts[2], &accounts[3], &accounts[4])?;
    let approvers = [
        key(data, 1)?,
        key(data, 33)?,
        key(data, 65)?,
        key(data, 97)?,
    ];
    valid_keys(&approvers, admin.key)?;
    let route: [u8; 32] = bytes(data, 129)?;
    require(
        route != [0; 32] && route == cfg.domain && cfg.max_amount >= HOUR_LIMIT,
        Error::Config,
    )?;
    let (funding, bump) = Pubkey::find_program_address(&[b"reserve", state.key.as_ref()], program);
    Reserve {
        core_program: *accounts[4].key,
        core_config: *accounts[2].key,
        mint: cfg.mint,
        administrator: *admin.key,
        funding,
        route,
        approvers,
        epoch: 1,
        max_swap: cfg.max_amount,
        max_gross: HOUR_LIMIT,
        pending: 0,
        paused: false,
        bump,
        governance_after: 0,
        governance_limit: 0,
        governance_epoch: 0,
        governance_kind: 0,
        governance_nonce: 0,
        pending_nonce: 0,
        next_approvers: [Pubkey::default(); 4],
    }
    .encode(&mut state.try_borrow_mut_data()?);
    Ok(())
}

fn fund(program: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    require(
        accounts.len() == 18 && data.len() == 210 && data[0] == 1 && data[165] == 1,
        Error::Instruction,
    )?;
    let state = &accounts[0];
    let payer = &accounts[1];
    let funding = &accounts[2];
    let mut reserve = checked(state, program)?;
    require(!reserve.paused, Error::Paused)?;
    quorum(&reserve, &accounts[15..18])?;
    single_instruction(program, accounts, data, &accounts[13])?;
    require(
        payer.is_signer
            && payer.is_writable
            && payer.key == &key(data, 81)?
            && payer.owner == &system_program::id()
            && payer.data_is_empty()
            && payer.key != &reserve.administrator
            && !reserve.approvers.contains(payer.key),
        Error::Authority,
    )?;
    require(
        funding.key == &reserve.funding
            && funding.owner == &system_program::id()
            && funding.data_is_empty()
            && !funding.executable
            && funding.is_writable,
        Error::Authority,
    )?;
    require(
        accounts[3].key == &reserve.core_config
            && accounts[10].key == &reserve.core_program
            && accounts[9].key == &reserve.mint,
        Error::Config,
    )?;
    let cfg = core_config(&accounts[3], &accounts[9], &accounts[10])?;
    require(
        cfg.domain == reserve.route && cfg.max_amount == reserve.max_swap,
        Error::Config,
    )?;
    let (funder, bump) = Pubkey::find_program_address(&[b"reserve", state.key.as_ref()], program);
    require(
        funding.key == &funder && bump == reserve.bump,
        Error::Authority,
    )?;
    let ata = associated_address(funding.key, &cfg.mint);
    require(
        accounts[6].key == &ata
            && accounts[8].key == &ata
            && accounts[6].owner == &spl_token::id()
            && accounts[6].data_len() == TokenAccount::LEN,
        Error::Account,
    )?;
    let source = TokenAccount::unpack(&accounts[6].try_borrow_data()?)?;
    require(
        source.owner == *funding.key
            && source.mint == cfg.mint
            && source.state == AccountState::Initialized
            && source.delegate == COption::None
            && source.close_authority == COption::None
            && source.is_native == COption::None,
        Error::Account,
    )?;
    require(
        accounts[11].key == &spl_token::id()
            && accounts[11].executable
            && accounts[12].key == &system_program::id()
            && accounts[12].executable,
        Error::Account,
    )?;
    let amount = num(data, 65)?;
    let id: [u8; 32] = bytes(data, 1)?;
    let approval_epoch = epoch(data, 166)?;
    let approval_expiry = num(data, 170)?;
    let proof: [u8; 32] = bytes(data, 178)?;
    let timestamp = now()?;
    require(approval_epoch == reserve.epoch, Error::Epoch)?;
    require(
        approval_expiry > timestamp
            && approval_expiry <= timestamp.checked_add(APPROVAL_TTL).ok_or(Error::Deadline)?
            && approval_expiry <= num(data, 73)?,
        Error::Deadline,
    )?;
    require(proof != [0; 32], Error::Replay)?;
    require(
        amount > 0 && amount <= reserve.max_swap && source.amount >= amount,
        Error::Amount,
    )?;
    require(reserve.pending < MAX_PENDING, Error::Capacity)?;
    let proof_account = &accounts[14];
    let (proof_address, proof_bump) =
        Pubkey::find_program_address(&[b"proof", state.key.as_ref(), &proof], program);
    require(proof_account.key == &proof_address, Error::Account)?;
    require(
        proof_account.owner == &system_program::id()
            && proof_account.data_is_empty()
            && !proof_account.executable,
        Error::Replay,
    )?;
    let cutoff = timestamp.saturating_sub(WINDOW);
    let mut gross = 0u64;
    let mut vacant = None;
    {
        let journal = state.try_borrow_data()?;
        for i in 0..CAPACITY {
            let offset = HEADER_LEN + i * ENTRY_LEN;
            let status = journal[offset + 48];
            require(status <= 2, Error::Account)?;
            let at = num(&journal, offset + 40)?;
            if status != 0 && (timestamp < WINDOW || at > cutoff) {
                gross = gross
                    .checked_add(num(&journal, offset + 32)?)
                    .ok_or(Error::Budget)?;
            }
            if (status == 0 || (status == 2 && timestamp >= WINDOW && at <= cutoff))
                && vacant.is_none()
            {
                vacant = Some(i);
            }
        }
    }
    require(
        gross.checked_add(amount).ok_or(Error::Budget)? <= reserve.max_gross,
        Error::Budget,
    )?;
    let index = vacant.ok_or(Error::Capacity)?;
    let funding_lamports = funding.lamports();
    let rent = Rent::get()?;
    let creation_rent = rent
        .minimum_balance(ESCROW_LEN)
        .saturating_sub(accounts[4].lamports())
        .checked_add(
            rent.minimum_balance(TokenAccount::LEN)
                .saturating_sub(accounts[5].lamports()),
        )
        .ok_or(Error::Amount)?;
    if creation_rent > 0 {
        invoke(
            &system_instruction::transfer(payer.key, funding.key, creation_rent),
            &[payer.clone(), funding.clone(), accounts[12].clone()],
        )?;
    }
    let proof_bump_bytes = [proof_bump];
    let proof_seeds = &[
        b"proof".as_ref(),
        state.key.as_ref(),
        &proof,
        &proof_bump_bytes,
    ];
    allocate_proof(payer, proof_account, &accounts[12], program, proof_seeds)?;
    {
        let mut receipt = proof_account.try_borrow_mut_data()?;
        put(&mut receipt, 0, PROOF_MAGIC);
        put(&mut receipt, 8, state.key.as_ref());
        put(&mut receipt, 40, &proof);
        put(&mut receipt, 72, &id);
        put(&mut receipt, 104, &reserve.epoch.to_le_bytes());
        put(&mut receipt, 108, &timestamp.to_le_bytes());
    }
    reserve.pending = reserve.pending.checked_add(1).ok_or(Error::Capacity)?;
    {
        let mut journal = state.try_borrow_mut_data()?;
        reserve.encode(&mut journal);
        let offset = HEADER_LEN + index * ENTRY_LEN;
        put(&mut journal, offset, &id);
        put(&mut journal, offset + 32, &amount.to_le_bytes());
        put(&mut journal, offset + 40, &timestamp.to_le_bytes());
        journal[offset + 48] = 1;
    }
    let metas = vec![
        AccountMeta::new_readonly(*accounts[3].key, false),
        AccountMeta::new(*accounts[4].key, false),
        AccountMeta::new(*accounts[5].key, false),
        AccountMeta::new(*funding.key, true),
        AccountMeta::new(*accounts[6].key, false),
        AccountMeta::new_readonly(*accounts[7].key, false),
        AccountMeta::new_readonly(*accounts[8].key, false),
        AccountMeta::new_readonly(*accounts[9].key, false),
        AccountMeta::new_readonly(*accounts[11].key, false),
        AccountMeta::new_readonly(*accounts[12].key, false),
    ];
    let ix = Instruction {
        program_id: reserve.core_program,
        accounts: metas,
        data: data[..166].to_vec(),
    };
    let bump_bytes = [reserve.bump];
    let seeds = &[b"reserve".as_ref(), state.key.as_ref(), &bump_bytes];
    invoke_signed(
        &ix,
        &[
            accounts[3].clone(),
            accounts[4].clone(),
            accounts[5].clone(),
            funding.clone(),
            accounts[6].clone(),
            accounts[7].clone(),
            accounts[8].clone(),
            accounts[9].clone(),
            accounts[11].clone(),
            accounts[12].clone(),
            accounts[10].clone(),
        ],
        &[seeds],
    )?;
    require(funding.lamports() == funding_lamports, Error::Amount)
}

fn pause(program: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    require(accounts.len() == 3 && data.len() == 1, Error::Instruction)?;
    let mut reserve = checked(&accounts[0], program)?;
    require(
        accounts[1].key == &reserve.administrator && accounts[1].is_signer,
        Error::Authority,
    )?;
    single_instruction(program, accounts, data, &accounts[2])?;
    reserve.paused = true;
    reserve.clear_governance();
    reserve.encode(&mut accounts[0].try_borrow_mut_data()?);
    Ok(())
}
fn reconcile(program: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    require(accounts.len() == 2 && data.len() == 33, Error::Instruction)?;
    let mut reserve = checked(&accounts[0], program)?;
    let escrow = &accounts[1];
    require(
        escrow.owner == &reserve.core_program && !escrow.executable,
        Error::Account,
    )?;
    let core = Escrow::decode(&escrow.try_borrow_data()?)?;
    let id: [u8; 32] = bytes(data, 1)?;
    require(
        core.config == reserve.core_config
            && core.funder == reserve.funding
            && core.id == id
            && core.direction == 1,
        Error::Config,
    )?;
    require(core.state == 2 || core.state == 3, Error::NotSettled)?;
    let expected = Pubkey::find_program_address(
        &[
            b"swap",
            reserve.core_config.as_ref(),
            reserve.funding.as_ref(),
            &id,
        ],
        &reserve.core_program,
    )
    .0;
    require(escrow.key == &expected, Error::Account)?;
    let mut journal = accounts[0].try_borrow_mut_data()?;
    let mut found = None;
    for i in 0..CAPACITY {
        let offset = HEADER_LEN + i * ENTRY_LEN;
        if journal[offset + 48] == 1 && bytes::<32>(&journal, offset)? == id {
            found = Some(offset);
            break;
        }
    }
    let offset = found.ok_or(Error::AlreadySettled)?;
    reserve.pending = reserve.pending.checked_sub(1).ok_or(Error::Account)?;
    reserve.encode(&mut journal);
    journal[offset + 48] = 2;
    Ok(())
}
fn governance_request(
    program: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> Result<Reserve, ProgramError> {
    require(accounts.len() == 6, Error::Instruction)?;
    let reserve = checked(&accounts[0], program)?;
    require(
        accounts[1].key == &reserve.administrator && accounts[1].is_signer,
        Error::Authority,
    )?;
    quorum(&reserve, &accounts[3..6])?;
    single_instruction(program, accounts, data, &accounts[2])?;
    require(epoch(data, 1)? == reserve.epoch, Error::Epoch)?;
    require(num(data, 5)? == reserve.governance_nonce, Error::Nonce)?;
    require(reserve.governance_kind == 0, Error::Governance)?;
    Ok(reserve)
}
fn queue_governance(reserve: &mut Reserve, kind: u8) -> ProgramResult {
    reserve.pending_nonce = reserve.governance_nonce;
    reserve.governance_nonce = reserve
        .governance_nonce
        .checked_add(1)
        .ok_or(Error::Nonce)?;
    reserve.governance_kind = kind;
    reserve.governance_epoch = reserve.epoch;
    reserve.governance_after = now()?.checked_add(ADMIN_DELAY).ok_or(Error::Deadline)?;
    Ok(())
}
fn governance_execution(
    program: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
    kind: u8,
) -> Result<Reserve, ProgramError> {
    require(accounts.len() == 2 && data.len() == 5, Error::Instruction)?;
    let reserve = checked(&accounts[0], program)?;
    single_instruction(program, accounts, data, &accounts[1])?;
    require(
        epoch(data, 1)? == reserve.epoch && reserve.governance_epoch == reserve.epoch,
        Error::Epoch,
    )?;
    require(reserve.governance_kind == kind, Error::Governance)?;
    require(
        reserve.governance_after > 0 && now()? >= reserve.governance_after,
        Error::Deadline,
    )?;
    Ok(reserve)
}
fn schedule_rotation(program: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    require(data.len() == 141, Error::Instruction)?;
    let mut reserve = governance_request(program, accounts, data)?;
    let keys = [
        key(data, 13)?,
        key(data, 45)?,
        key(data, 77)?,
        key(data, 109)?,
    ];
    valid_keys(&keys, &reserve.administrator)?;
    require(keys != reserve.approvers, Error::Rotation)?;
    reserve.next_approvers = keys;
    queue_governance(&mut reserve, 1)?;
    reserve.encode(&mut accounts[0].try_borrow_mut_data()?);
    Ok(())
}
fn execute_rotation(program: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    let mut reserve = governance_execution(program, accounts, data, 1)?;
    valid_keys(&reserve.next_approvers, &reserve.administrator)?;
    reserve.epoch = reserve.epoch.checked_add(1).ok_or(Error::Epoch)?;
    reserve.approvers = reserve.next_approvers;
    reserve.clear_governance();
    reserve.encode(&mut accounts[0].try_borrow_mut_data()?);
    Ok(())
}
fn schedule_limit(program: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    require(data.len() == 21, Error::Instruction)?;
    let mut reserve = governance_request(program, accounts, data)?;
    let limit = num(data, 13)?;
    require(limit > 0 && limit != reserve.max_gross, Error::Amount)?;
    reserve.governance_limit = limit;
    queue_governance(&mut reserve, 2)?;
    reserve.encode(&mut accounts[0].try_borrow_mut_data()?);
    Ok(())
}
fn execute_limit(program: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    let mut reserve = governance_execution(program, accounts, data, 2)?;
    require(reserve.governance_limit > 0, Error::Amount)?;
    reserve.max_gross = reserve.governance_limit;
    reserve.clear_governance();
    reserve.encode(&mut accounts[0].try_borrow_mut_data()?);
    Ok(())
}
fn schedule_resume(program: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    require(data.len() == 13, Error::Instruction)?;
    let mut reserve = governance_request(program, accounts, data)?;
    require(reserve.paused, Error::Paused)?;
    queue_governance(&mut reserve, 3)?;
    reserve.encode(&mut accounts[0].try_borrow_mut_data()?);
    Ok(())
}
fn execute_resume(program: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    let mut reserve = governance_execution(program, accounts, data, 3)?;
    require(reserve.paused, Error::Paused)?;
    reserve.paused = false;
    reserve.clear_governance();
    reserve.encode(&mut accounts[0].try_borrow_mut_data()?);
    Ok(())
}
fn cancel_governance(program: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    require(accounts.len() == 3 && data.len() == 1, Error::Instruction)?;
    let mut reserve = checked(&accounts[0], program)?;
    require(
        accounts[1].key == &reserve.administrator && accounts[1].is_signer,
        Error::Authority,
    )?;
    single_instruction(program, accounts, data, &accounts[2])?;
    require(reserve.governance_kind != 0, Error::Governance)?;
    reserve.clear_governance();
    reserve.encode(&mut accounts[0].try_borrow_mut_data()?);
    Ok(())
}
pub fn process_instruction(
    program: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    match data.first() {
        Some(0) => init(program, accounts, data),
        Some(1) => fund(program, accounts, data),
        Some(2) => pause(program, accounts, data),
        Some(3) => reconcile(program, accounts, data),
        Some(4) => execute_resume(program, accounts, data),
        Some(5) => schedule_rotation(program, accounts, data),
        Some(6) => execute_rotation(program, accounts, data),
        Some(7) => schedule_limit(program, accounts, data),
        Some(8) => execute_limit(program, accounts, data),
        Some(9) => schedule_resume(program, accounts, data),
        Some(10) => cancel_governance(program, accounts, data),
        _ => Err(Error::Instruction.into()),
    }
}
