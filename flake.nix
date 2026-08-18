{
  description = "koma - reproducible builds, nix run, and NixOS/home-manager installs";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      # Linux only for now: both packages are built and run here on x86_64-linux,
      # and CI exercises aarch64-linux too. macOS uses the system WebKit instead
      # of webkitgtk for the `gui` feature and hasn't been verified.
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;

      version = "0.3.20";

      mkPackages = system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          inherit (pkgs) lib;

          # `koma-extension` (src-extension) is excluded from the cargo workspace but is
          # a path dependency of `agent`, so it must ship in the build sandbox. `src-misc`,
          # `src-internet`, `src-security` and `models.json` are pulled in unconditionally
          # at compile time via `include_dir!`/`include_str!`, independent of any feature flag.
          commonFileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./models.json
            ./src-agent
            ./src-extension
            ./src-misc
            ./src-internet
            ./src-security
          ];

          commonSrc = lib.fileset.toSource {
            root = ./.;
            fileset = commonFileset;
          };

          # The `gui` feature additionally embeds the built `src-webgui/dist` via
          # `include_dir!`; `src-agent/build.rs` runs `npm install && npm run build`
          # itself when the feature is enabled, so the frontend sources must reach
          # the sandbox too.
          guiSrc = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.union commonFileset ./src-webgui;
          };

          # `npm install` inside build.rs has no network access in the Nix sandbox.
          # Pre-fetch the exact tarballs `package-lock.json` pins into a local cache
          # and point npm at it in offline mode so that install step still works.
          webguiNpmDeps = pkgs.fetchNpmDeps {
            name = "koma-webgui-npm-deps";
            src = ./src-webgui;
            hash = "sha256-2xnVtDzf0csUfrjgKecGOc7HtVg7GL9QD3fuCIIgnHo=";
          };

          # `useSystemOpenssl = true` links the system OpenSSL instead of compiling
          # reqwest's vendored copy from source. `openssl-sys` honours
          # `OPENSSL_NO_VENDOR` even when a dependency enables the `vendored`
          # feature, so this needs no Cargo.toml change. Off by default: koma
          # itself opted into `native-tls-vendored`, and this flake should build
          # exactly the binary upstream's own build produces unless told otherwise.
          mkKoma = lib.makeOverridable
            ({ pname
             , src
             , cargoBuildFlags
             , extraNativeBuildInputs ? [ ]
             , extraBuildInputs ? [ ]
             , env ? { }
             , preBuild ? ""
             , useSystemOpenssl ? false
             }:
              pkgs.rustPlatform.buildRustPackage {
                inherit pname version src cargoBuildFlags preBuild;

                cargoLock.lockFile = ./Cargo.lock;

                # Vendored OpenSSL (reqwest's native-tls-vendored) compiles from
                # source and needs perl; rusqlite (bundled) and the tree-sitter
                # grammars need a C compiler, which stdenv already provides.
                nativeBuildInputs = extraNativeBuildInputs
                  ++ lib.optionals (!useSystemOpenssl) [ pkgs.perl ]
                  ++ lib.optionals useSystemOpenssl [ pkgs.pkg-config ];
                buildInputs = extraBuildInputs
                  ++ lib.optionals useSystemOpenssl [ pkgs.openssl ];

                env = env // lib.optionalAttrs useSystemOpenssl {
                  OPENSSL_NO_VENDOR = "1";
                };

                doCheck = false;

                meta = {
                  description = "Fast, native AI coding agent for the terminal";
                  homepage = "https://koma.run";
                  license = lib.licenses.asl20;
                  mainProgram = "koma";
                  platforms = lib.platforms.unix;
                };
              });
        in
        {
          default = mkKoma {
            pname = "koma";
            src = commonSrc;
            cargoBuildFlags = [ "--no-default-features" "-p" "agent" ];
          };

          gui = mkKoma {
            pname = "koma-gui";
            src = guiSrc;
            cargoBuildFlags = [ "--features" "gui" "-p" "agent" ];
            extraNativeBuildInputs = [ pkgs.nodejs_24 pkgs.pkg-config pkgs.wrapGAppsHook3 ];
            extraBuildInputs = [ pkgs.webkitgtk_4_1 pkgs.gtk3 pkgs.libsoup_3 ];
            env = {
              npm_config_offline = "true";
              npm_config_cache = "${webguiNpmDeps}";
            };
            # `build.rs` runs `npm install && npm run build` itself once cargo
            # reaches `agent`'s build script, but npm packages ship `#!/usr/bin/env
            # node` shebangs and the sandbox has no `/usr/bin/env`. Pre-install here
            # and patch the shebangs so build.rs's own `npm install` is a no-op that
            # leaves the already-patched scripts alone.
            preBuild = ''
              (
                cd src-webgui
                HOME=$TMPDIR npm install
                patchShebangs node_modules
              )
            '';
          };
        };
    in
    {
      packages = forAllSystems mkPackages;

      devShells = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          packages = self.packages.${system};
        in
        {
          default = pkgs.mkShell {
            inputsFrom = [ packages.default packages.gui ];
            packages = with pkgs; [ rustfmt clippy rust-analyzer ];
          };
        });

      checks = forAllSystems (system: {
        default = self.packages.${system}.default;
        gui = self.packages.${system}.gui;
      });
    };
}
