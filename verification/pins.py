from pathlib import Path

PROGRAM_SHA256 = 'b5744350d710b6ac8beb293d8994610e84fa9060177b67bdef1cb0b922e3d1fc'
PROGRAM_BYTES = 147496
BUILD_ROOT = '/var/tmp/rgn-spl-combined-staging-20261123'
MAINNET_GENESIS = '5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d'
MINT = 'Cn7pt6uUQt6qdYSASpWgvQHm4D9aStaujRu7AYMABray'
COLD_ADMIN = '4ea15QbNLeGHTjkDyqrM9qiAG6o46fCXWAZbkxEbkT1R'
BASE_IMAGE = 'solanafoundation/solana-verifiable-build@sha256:12fd4c0a0790f0fc41ef74b0cdb6bccc167ba6137eb1adb749bac649481c86bd'
AGAVE_ARCHIVE = 'solana-release-x86_64-unknown-linux-gnu.tar.bz2'
AGAVE_URL = 'https://github.com/anza-xyz/agave/releases/download/v4.4.1/' + AGAVE_ARCHIVE
AGAVE_SHA256 = '5b0f0fc7e4eedace05f6455de00aa1f2f3d1642068f1e8d50760ba49112c3941'
PLATFORM_ARCHIVE = 'platform-tools-linux-x86_64.tar.bz2'
PLATFORM_URL = 'https://github.com/anza-xyz/platform-tools/releases/download/v1.57/' + PLATFORM_ARCHIVE
PLATFORM_SHA256 = 'b0f7af104adf726fff2a6a09ea2eb2f2d2965c92295f4d7388c08d140e0c2b00'
TOOL_SHA256 = {
    'tools/solana-release/bin/cargo-build-sbf': 'a5ca2a5f9489656171d6ec8308cf77197ba99452d9672496757880d66870e98f',
    'build-tools/v1.57/platform-tools/rust/bin/rustc': 'f7a4ec1b62e27fa990de4c9bd3c38eb7a5ba0a4f41b2e0929a0631c68c9c6e78',
    'build-tools/v1.57/platform-tools/rust/bin/cargo': 'c13a2c6dc68b95a0fd494d65a299264e4ad027544759a7291f6ccfcda4264603',
    'build-tools/v1.57/platform-tools/llvm/bin/clang': '6672ea42140527104c230c995992edf7147308a370ae3e52163821896d9b8c1a',
}
SOURCE_SHA256 = {
    'Cargo.toml': 'f8ad454618eaca7200f86c6b36f81478fac51241f544bf9f4258196f4a6f08f4',
    'Cargo.lock': 'a2f18849759fcd65f947d93079cae0144effa959d4f189368cee85df4fe7a460',
    'src/lib.rs': 'ae0661ef2ed8644787ea8b5b9081143f7c811cb5b578b5c67abac347248747c6',
    'reserve/Cargo.toml': '705eddf86c5f0ca02d26c81cd8579c2008c97710429db2858b99895ad53e0220',
    'reserve/Cargo.lock': '4a4243886c55f90c5c103b50fd79c6668796ad53e50f453626160c49213327eb',
    'reserve/src/lib.rs': 'f542a22810035c5ea2743d67de7263049c4b23e821d9aac5522e9d31329c3bb1',
    'combined/Cargo.toml': '61db6954b2f9e4ff7612d75dc452a1a5e57ed80327e0f7310dce01c9cbb5272a',
    'combined/Cargo.lock': '8c11a427331163761132e9427d5472f60c4a9ce07ae8135a87cecd3b7f8c3dce',
    'combined/src/lib.rs': '486a1f95918c323357adc2f20f37e255a91267d1c4e0cd0a5322a8f5065e5e3e',
}
PUBLIC_SOURCE_SHA256 = dict(SOURCE_SHA256)
PUBLIC_SOURCE_SHA256['combined/Cargo.toml'] = '45e03e7b2734d2ad04d98d844b9445e1f1f24e7fd1b683d710c0ba9797174aad'


def qualified_source():
    return Path(__file__).resolve().parents[2] / 'rgn-spl-combined-20261123' / 'solana'
