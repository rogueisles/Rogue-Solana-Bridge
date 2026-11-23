# Reproducible Solana builds

The build image pins the base image, Agave binaries, platform tools and all Cargo sources by SHA256. Wallets and deployment settings are not part of the build context.

Prepare a new context outside the source checkout. The script copies only the nine Rust and Cargo files and the public build helpers, then downloads the two checksum-verified compiler archives.

```
python3 verification/prepare_context.py /tmp/rgn-solana-build-context
docker build --platform linux/amd64 -f /tmp/rgn-solana-build-context/Dockerfile.verifiable -t rgn-solana-build /tmp/rgn-solana-build-context
```

Use the image's immutable registry digest when submitting source verification. The exact source checkout must contain the three crates at its root: `Cargo.toml`, `src`, `reserve` and `combined`.

With solana-verify 0.5.2, use the following build parameters:

```
solana-verify build . --workspace-path combined --library-name rgn_spl_bridge --base-image IMAGE_DIGEST --arch v0 --cargo-build-sbf-args="--tools-version v1.57 --skip-tools-install --no-rustup-override --jobs 1 --offline"
solana-verify get-executable-hash combined/target/deploy/rgn_spl_bridge.so
```

Version 0.5.2 supplies Cargo's locked-dependency flag itself. Do not append it a second time. The expected verifier hash is `77f23ef35a43c3f8a3be7f31ff73de1284aed60436eccbd50097ee1a1a8a851c`.
