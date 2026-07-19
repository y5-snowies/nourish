# Rust libinput bindings

[![Build Status](https://img.shields.io/github/actions/workflow/status/Smithay/input.rs/ci.yml?branch=master&logo=github-actions&logoColor=white&style=for-the-badge)](https://github.com/Smithay/input.rs/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/input.svg?logo=rust&style=for-the-badge)](https://crates.io/crates/input)
[![License](https://img.shields.io/crates/l/input.svg?style=for-the-badge)](https://crates.io/crates/input)
[![Docs](https://img.shields.io/docsrs/input?style=for-the-badge)](https://docs.rs/input)

[libinput](https://wayland.freedesktop.org/libinput/doc/latest/) bindings for [Rust](https://www.rust-lang.org)

These bindings closely follow libinput's concepts and it's original API.
Please refer to the [libinput documentation](https://wayland.freedesktop.org/libinput/doc/latest/) to understand the general structure and concepts.

**Note:** Due to a bug within libinput, these bindings are *not* compatible with libinput 1.19.**0**. Please use the fixed 1.19.**1** version.

## Usage

Add to your `Cargo.toml`:

```toml
input = "0.8"
```

Install the libinput dev dependencies:

Ubuntu:
```
apt-get install libinput-dev
```
Fedora
```
dnf install libinput-devel
```

Configure and run event loop:

```rust
use input::{Libinput, LibinputInterface};
use libc::{O_ACCMODE, O_RDONLY, O_RDWR, O_WRONLY};
use std::fs::{File, OpenOptions};
use std::os::unix::{fs::OpenOptionsExt, io::OwnedFd};
use std::path::Path;

struct Interface;

impl LibinputInterface for Interface {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        OpenOptions::new()
            .custom_flags(flags)
            .read((flags & O_ACCMODE == O_RDONLY) | (flags & O_ACCMODE == O_RDWR))
            .write((flags & O_ACCMODE == O_WRONLY) | (flags & O_ACCMODE == O_RDWR))
            .open(path)
            .map(|file| file.into())
            .map_err(|err| err.raw_os_error().unwrap())
    }
    fn close_restricted(&mut self, fd: OwnedFd) {
        drop(File::from(fd));
    }
}

fn main() {
    let mut input = Libinput::new_with_udev(Interface);
    input.udev_assign_seat("seat0").unwrap();
    loop {
        input.dispatch().unwrap();
        for event in &mut input {
            println!("Got event: {:?}", event);
        }
    }
}
```
