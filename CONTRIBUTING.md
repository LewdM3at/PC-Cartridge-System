# How to contribute

Thank you for being interested in contributing to this project.

Please keep in mind below rules before committing changes and doing pull request.

## Project designation

This project is meant to be a fun way to store your digital game library on a separate drive to free up space on your computer, by also simulating cartridge system used in many consoles throughout history. 

We are glad it sparked conversations and movements, however contrary to some misinformation online this project is NOT about:

* Game preservation
* Game ownership
* Fighting against big corporations

This is Free Open Source project and this repository contains scripts to use with your extra SSDs (and other storage mediums) to launch games on various operating systems and platforms. 

Please keep that in mind when contributing features.

## AI usage

If you decide on using AI agent for code, consider important rules:

* Make sure you read and understand the output of your AI agent and code it provides you
* Do not blindly commit AI code without testing it first
* Please do not use AI to write entire code for you - it should be assistance, not developer replacement
* Be honest about your AI usage and mention or co-author it in your pull requests

This is a hobby project meant for people, by people, to try and enjoy creating their own things - it is not a place for testing and improving your AI agents

## Issues and pull requests

When making a pull request, it is best practice to first open an issue. You are welcome to work on any, currently open issue, while keeping in mind:

* Bug reports should be exclusively human made and contain detailed information on how to replicate them
* Features should adhere to the project designation
* Pull requests should contain detailed list of changes and related issue number
* Pull requests should contain a single topic as far as possible. Don't create pull requests where you add 4 features and fixed 6 bugs at the same time.
* Your code should be easily readable for everyone involved
* Please do not pull request to main branch, instead use dev branch

## Checking your work before CI does

The three crates are checked separately, and `RUSTFLAGS=-D warnings` is what CI
uses, so a warning is a failed build there:

```bash
cargo fmt --manifest-path core/Cargo.toml --check
RUSTFLAGS=-D warnings cargo clippy --manifest-path core/Cargo.toml --all-targets
RUSTFLAGS=-D warnings cargo test --manifest-path core/Cargo.toml
node tools/check-dom-ids.mjs      # every id the frontend reaches for exists
node tools/check-versions.mjs     # all fourteen version sites agree
```

### Checking the Windows side from Linux

**Do this if you touch anything behind `#[cfg(windows)]`.** This project has been
caught twice by code that compiles on Linux and does not exist on Windows — once
by a `windows-sys` dependency that was never declared, and once by two of its
features that were used without being enabled. Neither is visible on Linux,
because the whole module is configured out.

```bash
sudo apt install gcc-mingw-w64-x86-64
rustup target add x86_64-pc-windows-gnu
RUSTFLAGS=-D warnings cargo clippy --manifest-path core/Cargo.toml \
  --target x86_64-pc-windows-gnu --all-targets
```

The GNU target rather than MSVC, and only because of the dependency tree: `ring`
arrives through `ureq`'s TLS and its build script wants a Microsoft compiler,
which stops an MSVC cross-check before it reaches any of this project's code. The
`cfg(windows)` paths and the `windows-sys` feature gates are identical either
way, which is what this is checking.

It catches real things. The `HANDLE` that `CreateToolhelp32Snapshot` returns is
an `isize` in `windows-sys` 0.52, not a pointer, so `is_null()` does not exist on
it — and the two calls beside each other fail differently, one with
`INVALID_HANDLE_VALUE` and one with a zero handle. None of that is knowable from
the Linux build.

## Final notes

Even if obvious, it should be stated that it is not given that this repository will be rigorously maintained. It is a hobby project meant for people to enjoy for free, for themselves. You are also free to fork this repository and do whatever you want with it, as per MIT license, but if you want to help with contributions, please keep in mind all the above guidelines.

Thank you so much for getting involved and for simply enjoying this project.
