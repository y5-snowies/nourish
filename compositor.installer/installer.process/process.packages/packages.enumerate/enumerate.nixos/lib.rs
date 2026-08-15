//! NixOS support — declarative, NOT a transactional install.
//!
//! NixOS is non-FHS: a prebuilt, dynamically-linked binary can't even find its ELF
//! interpreter (`/lib64/ld-linux-*.so`), let alone its libraries, so there is nothing to
//! `pacman -S`. The idiomatic fix is `programs.nix-ld` — it provides the interpreter at
//! the standard path and exposes the listed libraries via `NIX_LD_LIBRARY_PATH`. So on
//! NixOS the installer does NOT run a package command; it **prints a `configuration.nix`
//! snippet** (the runtime libs as nixpkgs attributes + nix-ld enablement) and tells the
//! user how to apply it. The package "names" here are therefore nixpkgs attribute names.
//! Pure std.

pub mod nixos;
pub use nixos::*;
