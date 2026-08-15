# environment/cross.compile/ — build the aarch64 binary on an x86_64 host

Compiling `y5_compositor` on the Raspberry Pi takes about an hour even incrementally.
This directory builds the same binary on the x86_64 host in **minutes** and leaves you a
file to copy over. It is self-contained: no container, and no access to the device — the
aarch64 headers and libraries live here, in `fedora44/`.

Nothing in `environment/` is modified or wrapped; `build.sh` here is a sibling of
`environment/build.sh` that keeps the same backend/profile mapping.

```
environment/cross.compile/
  sysroot.sh    # populate fedora<rel>/ from Fedora aarch64 rpms   (run once)
  build.sh      # cross-compile   [winit|udev|native] [release-fast|release]
  fedora44/     # the sysroot: aarch64 headers, .so files, .pc files (~690 MB)
  .rpm.cache/   # downloaded rpms, reused by sysroot.sh
  .shim/        # generated linker/cc wrapper
```

## One-time host setup

```bash
sudo dnf install -y gcc-aarch64-linux-gnu binutils-aarch64-linux-gnu rustup
rustup-init -y --no-modify-path --profile minimal
rustup target add aarch64-unknown-linux-gnu
./sysroot.sh 44          # populates ./fedora44 — a few minutes, ~690 MB
```

`rustup` is required because Fedora's packaged `rustc` ships `rust-std` only for the
wasm/uefi/none targets — there is no `aarch64-unknown-linux-gnu` std in the distro
toolchain. The host `cargo` is left alone; `build.sh` puts `~/.cargo/bin` first only for
its own invocation.

## Building

```bash
./build.sh                    # udev + release-fast (the default)
./build.sh udev release       # fat LTO — the cross equivalent of build-optimized.sh
./build.sh winit              # nested backend instead of DRM/KMS
```

Copying the binary to the device is a separate, manual step. Once it is there, give it
`sudo setcap cap_sys_nice+ep <path>` — the capability cannot be applied to a foreign-arch
binary on this host, and copying would drop it regardless; without it the compositor's
`priority="auto"` falls back to rtkit over D-Bus.

Output goes to `~/.cache/y5-cross/target/aarch64-unknown-linux-gnu/<profile>/y5_compositor`
and the path is printed on stdout, so `BIN="$(./build.sh udev)"` works the same way
it does with `environment/build.sh`.

Cross builds use their own target dir on purpose: cargo keeps build-script and proc-macro
artifacts per *host*, so sharing `target/` with host builds would make every switch
between them a near-full rebuild.

## The release number is the ABI contract

`sysroot.sh 44` fetches **Fedora 44** aarch64 packages, so the binary links against that
release's glibc and sonames — the current output requires `GLIBC_2.43` and
`libavcodec.so.62`, `libinput.so.10`, `libseat.so.1`. It runs on a Fedora 44 device and
will *not* start on an older release. If the Pi moves to Fedora 45, run `./sysroot.sh 45`
and build with `Y5_SYSROOT=$PWD/fedora45`.

This is why the sysroot is built from packages of a pinned release rather than from
whatever the host happens to have installed.

## Adding a dependency

If a new crate needs a system library, add its `-devel` package to `PKGS` in `sysroot.sh`
and re-run it (incremental — the rpm cache is reused). The script ends with two checks: a
hard gate on link inputs (`crt1.o`, `libc.so`, `libgcc_s.so`, the major `.so` files) and a
report of pkg-config coverage. A sysroot that fails the gate refuses to be used.

## Notes on the moving parts

- **Everything goes through one shim.** `.shim/aarch64-unknown-linux-gnu-cc` wraps
  `aarch64-linux-gnu-gcc` with `--sysroot`, `-L` and `-rpath-link`. rustc offers no way to
  inject `--sysroot` per link, and cc-rs needs the identical view, so both use the wrapper.
- **`RUSTFLAGS` is still off-limits.** It replaces `.cargo/config.toml` rustflags wholesale
  (dropping `-A warnings`). `Y5_CROSS_CPU` goes through the per-target env channel, which
  cargo *joins* with the config instead.
- **`PKG_CONFIG_LIBDIR` must never be empty.** pkg-config reads empty as "use built-in
  defaults" and silently answers with the host's x86_64 `.pc` files; `build.sh` fails
  instead.
- **Two sysroot subtleties**, both handled by `sysroot.sh` and both silent if missed:
  the `filesystem` rpm owns `/usr/lib64` and ships it mode 0555, so packages extracted
  after it cannot write their libraries there (the result looks complete — all headers,
  all `.pc` files, almost no `.so`); and `libgcc_s.so`, which rustc's unwinder always
  wants, ships in Fedora's `gcc` package rather than `libgcc`, so it is created as a shim.
