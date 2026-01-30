{
  description = "DOLI - VDF-based blockchain with Proof of Time (PoT) consensus";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        # Rust toolchain - latest stable for full compatibility
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" "clippy" ];
        };

        # Common build inputs for all platforms
        commonBuildInputs = with pkgs; [
          # GMP for rug crate (VDF big integer operations)
          gmp

          # RocksDB dependencies
          rocksdb

          # OpenSSL for network/crypto
          openssl

          # libp2p dependencies
          protobuf
        ];

        # Darwin-specific build inputs
        # Note: darwin.apple_sdk.frameworks has been removed in nixpkgs-unstable
        # The default SDK now handles frameworks automatically via $SDKROOT
        darwinBuildInputs = pkgs.lib.optionals pkgs.stdenv.isDarwin [
          pkgs.darwin.libiconv
        ];

        buildInputs = commonBuildInputs ++ darwinBuildInputs;

        nativeBuildInputs = with pkgs; [
          pkg-config
          clang
          llvmPackages.libclang
        ];

      in {
        devShells.default = pkgs.mkShell {
          inherit buildInputs nativeBuildInputs;

          packages = [
            rustToolchain
            pkgs.cargo-watch
            pkgs.cargo-edit
            pkgs.cargo-audit
          ];

          # Environment variables for linking
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          ROCKSDB_LIB_DIR = "${pkgs.rocksdb}/lib";
          OPENSSL_DIR = "${pkgs.openssl.dev}";
          OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";

          shellHook = ''
            echo "🚀 DOLI development environment loaded"
            echo "   Rust: $(rustc --version)"
            echo "   Cargo: $(cargo --version)"
            echo ""
            echo "Commands:"
            echo "   cargo build          - Build all crates"
            echo "   cargo test           - Run tests"
            echo "   cargo run -p node    - Run node"
            echo "   cargo run -p cli     - Run CLI"
          '';
        };

        # To enable `nix build`, add Cargo.lock to git and uncomment:
        # packages.default = pkgs.rustPlatform.buildRustPackage {
        #   pname = "doli";
        #   version = "0.1.0";
        #   src = ./.;
        #   cargoLock.lockFile = ./Cargo.lock;
        #   inherit buildInputs nativeBuildInputs;
        #   LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
        #   ROCKSDB_LIB_DIR = "${pkgs.rocksdb}/lib";
        # };
      }
    );
}
