{
  description = "Rust Development Shell";

  inputs = {
    nixpkgs.url      = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url  = "github:numtide/flake-utils";
    barretenberg-nix = {
      url = "git+ssh://git@github.com/satsbridge/barretenberg-nix.git";
    };
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, barretenberg-nix, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        barretenberg-pkg = barretenberg-nix.packages.${system}.default;
        noir-pkg = barretenberg-nix.packages.${system}.noir;
      in
      with pkgs;
      {
        # Override the default GCC environment with the Clang environment
        devShells.default = mkShell.override { stdenv = llvmPackages_latest.stdenv; } {
          nativeBuildInputs = [
            cmake
            pkg-config
            ninja
            llvmPackages_latest.bintools
          ];

          buildInputs = [
            barretenberg-pkg
            noir-pkg
            python310
            openssl
            go
            protobuf
            (
              rust-bin.fromRustupToolchainFile ./rust-toolchain.toml
            )
            llvmPackages_latest.openmp
          ];
          
          RUST_SRC_PATH = pkgs.rustPlatform.rustLibSrc;
          LIBCLANG_PATH = pkgs.lib.makeLibraryPath [ pkgs.llvmPackages_latest.libclang.lib ];

	  shellHook = ''
  		export CMAKE_C_FLAGS="-march=x86-64"
  		export CMAKE_CXX_FLAGS="-march=x86-64"
	  '';
        };
      }
    );
}
