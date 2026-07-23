{
  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
    nixpkgs.url = "github:NixOS/nixpkgs/26.05";
  };

  outputs =
    inputs:
    inputs.flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import inputs.nixpkgs {
          inherit system;

          config.allowUnfree = true;
        };
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "cart";
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
          src = ./.;

          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = with pkgs; [
            cmake
            installShellFiles
            perl
          ];

          env.SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";

          postInstall = ''
            out_dir=$(find target -type d -path '*/build/cart-*/out' | head -1)
            installShellCompletion --cmd cart \
              --bash "$out_dir/cart.bash" \
              --zsh  "$out_dir/_cart" \
              --fish "$out_dir/cart.fish" \
              --nushell "$out_dir/cart.nu"
          '';

          meta = {
            description = "Minecraft launcher and mod manager";
            mainProgram = "cart";
            platforms = pkgs.lib.platforms.unix;
          };
        };

        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            cargo
            claude-code
            clippy
            nixd
            nixfmt
            rustc
            rustfmt
            rust-analyzer
          ];
        };
      }
    );
}
