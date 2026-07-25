# Publishing

This workspace publishes six crates. Publish them in dependency order so Cargo
can verify each downstream package against the newly published registry
versions:

1. `tiff-core`
2. `geotiff-core`
3. `tiff-reader`
4. `tiff-writer`
5. `geotiff-reader`
6. `geotiff-writer`

Run the same local release checks before publishing:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test -p tiff-reader --no-default-features
cargo test -p tiff-writer --no-default-features
cargo test -p geotiff-reader --no-default-features
cargo test -p geotiff-reader --no-default-features --features cog
cargo test -p geotiff-reader --no-default-features --features cog-async
cargo doc --workspace --all-features --no-deps
./scripts/fetch-interoperability-corpus.sh --verify-only
./scripts/run-reference-parity.sh
./scripts/seed-fuzz-corpus.sh
git diff --exit-code -- fuzz/corpus
git status --short -- fuzz/corpus
cargo check --manifest-path fuzz/Cargo.toml --bins --locked
cargo clippy --manifest-path fuzz/Cargo.toml --bins --locked -- -D warnings
(
  cd fuzz
  cargo fuzz run tiff_open corpus/tiff_open -- -max_total_time=60
  cargo fuzz run geotiff_open corpus/geotiff_open -- -max_total_time=60
  cargo fuzz run geotiff_subifd_overviews -- -max_total_time=60
)
cargo package -p tiff-core
cargo package -p geotiff-core
```

After `tiff-core` and `geotiff-core` are published, run `cargo package` for
the dependent crates in the order above, then publish each crate with:

```sh
cargo publish -p <crate>
```

Cargo verifies package tarballs using registry dependencies rather than local
path dependencies, so dependent crates cannot complete a full `cargo package`
verification until the same-version internal dependencies are available on
crates.io.

Before those internal versions are live, you can still locally verify the
downstream tarballs with temporary patches:

```sh
cargo package -p tiff-reader \
  --config 'patch.crates-io.tiff-core.path="tiff-core"'
cargo package -p tiff-writer \
  --config 'patch.crates-io.tiff-core.path="tiff-core"'
cargo package -p geotiff-reader \
  --config 'patch.crates-io.geotiff-core.path="geotiff-core"' \
  --config 'patch.crates-io.tiff-core.path="tiff-core"' \
  --config 'patch.crates-io.tiff-reader.path="tiff-reader"'
cargo package -p geotiff-writer \
  --config 'patch.crates-io.geotiff-core.path="geotiff-core"' \
  --config 'patch.crates-io.tiff-core.path="tiff-core"' \
  --config 'patch.crates-io.tiff-writer.path="tiff-writer"'
```
