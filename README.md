# Rogue Solana Bridge

Contracts for exchanging existing RGN on Solana and native RGN on Rogue Network at a 1:1 token ratio. Liquidity comes from funded reserves. The bridge does not mint new tokens or change either network's consensus.

| Network | Asset |
| --- | --- |
| Rogue Network | Native RGN, chain ID 70088, 18 decimals |
| Solana mainnet | `Cn7pt6uUQt6qdYSASpWgvQHm4D9aStaujRu7AYMABray`, 6 decimals |

The Solana program combines token escrows and the reserve controller. Rogue Network uses `NativeRgnEscrow` and `QuorumRgnReserve`, built with Solidity 0.8.23 for Byzantium.

## Operation

A user locks funds with an exact recipient, amount, deadline and hash commitment. Three of four separate approval services must approve the matching destination payment. The contracts enforce the approval threshold, single-use commitments and spending limits. The receiving user claims the destination tokens, allowing the original deposit to settle into the opposite reserve. Expired escrows can be refunded under their on-chain deadlines.

The initial rolling limit is 10 million RGN per direction over 60 minutes. Administrative changes to limits, approval keys and funding resumption have a 24-hour delay. The cold administrator can pause new funding immediately. Existing claims and eligible refunds remain available.

Users pay normal transaction fees and their required Solana account deposits. These costs are separate from the 1:1 token principal. A bounded project gas wallet handles unattended final settlement. A swap requires available liquidity on the destination network.

## Source layout

* `src` contains the Solana token escrow.
* `reserve` contains the Solana reserve controller.
* `combined` is the combined Solana program.
* `native/contracts` contains the Rogue Network contracts.
* `native/compiler-input.json` records the exact Solidity build settings.
* `verification` contains the pinned Solana build recipe.

## Builds

Solana uses platform tools v1.57, the SBF v0 target, release optimization `s`, fat LTO and checked arithmetic. All dependency lockfiles are included. Reproducible builds use the absolute build paths and pinned compiler archives in `verification`.

The complete qualified Solana executable is 147,496 bytes with SHA256:

```
b5744350d710b6ac8beb293d8994610e84fa9060177b67bdef1cb0b922e3d1fc
```

The Solana verifier trims trailing zero bytes and reports:

```
77f23ef35a43c3f8a3be7f31ff73de1284aed60436eccbd50097ee1a1a8a851c
```

For Solidity, compile `native/compiler-input.json` using solc 0.8.23 with optimization enabled at 200 runs, Byzantium and metadata bytecode hashing disabled.

## Deployment

The contracts below are deployed on mainnet. The Solana program has no upgrade authority. Public swaps remain disabled while account initialization, production service qualification and limited mainnet swap and refund tests are completed.

| Network | Contract | Address |
| --- | --- | --- |
| Rogue Network | NativeRgnEscrow | `0x65D12f35ea4B09d7d1826e331494e319d165008b` |
| Rogue Network | QuorumRgnReserve | `0x5382180b0D608433e586DcF7a388Fe79cf84cBA1` |
| Solana | Combined escrow and reserve program | [E9UThbDmqpjFMbkVWqJXzgGrg5BYmwogtoFWs5D6RxRs](https://solscan.io/account/E9UThbDmqpjFMbkVWqJXzgGrg5BYmwogtoFWs5D6RxRs) |

The verified source is pinned to [commit 0448bd36](https://github.com/rogueisles/Rogue-Solana-Bridge/tree/0448bd36af1f279538f684a3a5cbbf4eddaac723). The [independent Solana verification record](https://verify.osec.io/job/281f7459-667a-4dc5-ae55-c2989f61884a) confirms that the published build matches the on-chain executable.

The public [verification build image](https://hub.docker.com/r/roguenetwork/solana-bridge-build) is `roguenetwork/solana-bridge-build:20261123-v1.57`, with immutable digest `sha256:8a912ba4fda10cb49350d7b63b04de1e95a7f8d671f6c808f3a085dd85fa0875`.

Source verification establishes that published source matches deployed code. It is separate from a security audit.

## License

MIT.
