# Fragment-shader-like graphics with Raspberry Pi Pico 2

## Generating a UF2 binary
Ensure that Rust is up-to-date, target support for `thumbv6m-none-eabi` is provided, and elf2uf2-rs is installed:
```
rustup self update
rustup update stable
rustup target add thumbv8m.main-none-eabihf
```

Execute `cargo run --release` to generate the UF2 binary at `target/thumbv6m-none-eabi/release/rusty-obegraensad.uf2`.
