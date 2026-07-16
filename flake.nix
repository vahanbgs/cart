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
        };
      in
      {
        # packages.default = pkgs.rustPlatform.buildRustPackage {
        #   pname = "cart";
        #   version = "0.1.0";
        #   src = ./.;

        #   cargoHash = "sha256-FWLHdHv+EJ5PYP1TTCN3G10RDDTGAnugBSQ2eYemKCs=";

        #   nativeBuildInputs = with pkgs; [
        #     installShellFiles
        #   ];

        #   postInstall = ''
        #     installShellCompletion --cmd cart \
        #       --bash target/*/build/cart-*/out/cart.bash \
        #       --zsh target/*/build/cart-*/out/_cart \
        #       --fish target/*/build/cart-*/out/cart.fish \
        #       --nushell target/*/build/cart-*/out/cart.nu
        #   '';
        # };

        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            cargo
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
