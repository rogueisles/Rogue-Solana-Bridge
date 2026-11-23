#![allow(unexpected_cfgs)]

use solana_program::{account_info::AccountInfo, entrypoint::ProgramResult, pubkey::Pubkey};

solana_program::entrypoint!(process_instruction);

pub fn process_instruction(
    program: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    if data.first() == Some(&rgn_spl_reserve::NAMESPACE) {
        rgn_spl_reserve::process_instruction(program, accounts, &data[1..])
    } else {
        rgn_spl_escrow::process_instruction(program, accounts, data)
    }
}
