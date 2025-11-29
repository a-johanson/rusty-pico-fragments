# Fragment-shader-like graphics with Raspberry Pi Pico 2

## Building
Ensure that Rust is up-to-date and that target support for `thumbv8m.main-none-eabihf` is provided:
```
rustup self update
rustup update
rustup target add thumbv8m.main-none-eabihf
```

Furthermore, ensure that `picotool` is in the PATH.

Execute `cargo run --release` to build the project and flash the resulting image onto a connected Raspberry Pi Pico 2 in BOOTSEL mode.
