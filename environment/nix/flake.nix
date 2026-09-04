{
  description = "y5 compositor host build environment (distro-agnostic alternative to install-deps.sh)";

  inputs = {
    # Unstable tracks current Rust; the nixos-25.11 release froze rustc at 1.91,
    # which is too old for the vendored bevy (needs >= 1.95).
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # Lets a Nix-environment process use the HOST GPU driver (NVIDIA) at runtime,
    # so the compositor can actually run from inside the shell. Requires --impure
    # (it auto-detects the host driver version) — nix.sh passes that.
    nixgl = {
      url = "github:nix-community/nixGL";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      nixgl,
    }:
    let
      inherit (nixpkgs) lib;

      # Build for every Linux system nixpkgs exposes (x86_64-linux, aarch64-linux, ...).
      systems = lib.intersectLists lib.systems.flakeExposed lib.platforms.linux;
      forAllSystems = lib.genAttrs systems;
      pkgsFor = forAllSystems (system: nixpkgs.legacyPackages.${system});
    in
    {
      formatter = forAllSystems (system: pkgsFor.${system}.nixfmt-rfc-style);

      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor.${system};

          # Host-GPU run wrapper. Auto-detecting the NVIDIA driver only works where the
          # host driver is visible to the Nix eval (i.e. NOT inside a GPU-less container),
          # otherwise it is null. Guard it so a null never breaks `nix develop` for builds.
          nixGLNvidia = nixgl.packages.${system}.nixGLNvidia or null;

          # Libraries the tree dlopen()s at runtime rather than linking directly
          # (wayland-rs, libxkbcommon, GL/Vulkan loaders). A hermetic mkShell does NOT
          # put buildInputs on the loader path, so these must go on LD_LIBRARY_PATH or
          # you get runtime errors like `NoWaylandLib`.
          runtimeLibs = [
            pkgs.wayland
            pkgs.libxkbcommon
            pkgs.libGL
            pkgs.libglvnd
            pkgs.vulkan-loader
            pkgs.libinput
            pkgs.seatd
          ];
        in
        {
          default = pkgs.mkShell {
            # Rust toolchain (stable, from nixpkgs — matches repo-root rust-toolchain.toml).
            packages = [
              pkgs.rustc
              pkgs.cargo
              pkgs.rust-analyzer
              pkgs.clippy
              pkgs.rustfmt
            ]
            # Host-GPU wrapper: prefix the run command with `nixGLNvidia` to bind the
            # host NVIDIA driver, e.g.  nixGLNvidia ./environment/run-host.sh winit debug.
            # Only present when the host driver was detectable (see note above).
            ++ lib.optional (nixGLNvidia != null) nixGLNvidia;

            # Point rust-analyzer / build scripts at the stdlib source.
            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";

            # Tools that run on the build host (mirrors install-deps.sh: clang + pkg-config).
            nativeBuildInputs = [
              pkgs.pkg-config
              pkgs.clang
              pkgs.rustPlatform.bindgenHook
              pkgs.protobuf # protoc (prost-build)
            ];

            # Devel libraries the tree links against (mirrors install-deps.sh).
            buildInputs = [
              # Wayland / smithay core
              pkgs.libinput
              pkgs.seatd # libseat
              pkgs.libxkbcommon
              pkgs.pixman
              pkgs.wayland
              pkgs.wayland-protocols
              pkgs.libgbm
              pkgs.libdisplay-info
              pkgs.systemd # libudev / libsystemd
              pkgs.dbus # libdbus-1 (libdbus-sys)
              pkgs.pam # libpam (session/login)

              # Graphics stack
              pkgs.libGL
              pkgs.libglvnd
              pkgs.ffmpeg # screen capture / video encode
              pkgs.libpulseaudio # libpulse (audio)

              # Diagnostics (egl-utils / mesa-demos equivalents; provides glxinfo + eglinfo)
              pkgs.mesa-demos

              # Developer-tool window (Tauri 2) GUI devel — needed to build the full
              # install bundle's dev-tool component (matches ci/Containerfile).
              pkgs.webkitgtk_4_1 # webkit2gtk4.1
              pkgs.libsoup_3 # libsoup3
              pkgs.gtk3
              pkgs.librsvg # librsvg2
              pkgs.libappindicator-gtk3

            ];

            shellHook = ''
              export LD_LIBRARY_PATH="${lib.makeLibraryPath runtimeLibs}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
              echo "y5 nix dev shell — build deps ready."
              echo "  build:  ./environment/build.sh udev release"
            ''
            + lib.optionalString (nixGLNvidia != null) ''
              echo "  run:    nixGLNvidia ./environment/run-host.sh winit debug"
              echo "          (nixGLNvidia binds the host NVIDIA driver for GPU at runtime)"
            ''
            + lib.optionalString (nixGLNvidia == null) ''
              echo "  run:    nixGLNvidia unavailable here (host NVIDIA driver not visible"
              echo "          to Nix eval — e.g. GPU-less container). Build only."
            '';
          };
        }
      );
    };
}
