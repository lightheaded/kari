{
  description = "kari — a Kanban board for Claude Code sessions; this flake builds the headless node";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  # The desktop app is a macOS bundle and comes from a GitHub release. This
  # flake builds `kari-node`, the headless node that a desktop app connects to
  # over an SSH port forward. A NixOS host can run it as a service:
  #
  #   systemd.services.kari-node = {
  #     wantedBy = [ "multi-user.target" ];
  #     path = [ claudePackage pkgs.git pkgs.jq pkgs.curl ];
  #     serviceConfig = {
  #       ExecStart = "${kari.packages.x86_64-linux.default}/bin/kari-node serve";
  #       User = "you";
  #       Restart = "on-failure";
  #     };
  #   };
  #
  # The node binds 127.0.0.1 only, so it needs no open port in the firewall.
  outputs =
    { nixpkgs, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        manifest = builtins.fromTOML (builtins.readFile ./Cargo.toml);
        kari-node = pkgs.rustPlatform.buildRustPackage {
          pname = "kari-node";
          version = manifest.workspace.package.version;
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          # The workspace also holds the Tauri app, which needs a desktop
          # toolchain. The node is a plain binary, so build that package alone.
          cargoBuildFlags = [
            "--package"
            "kari-cli"
          ];
          cargoTestFlags = [
            "--package"
            "kari-core"
            "--package"
            "kari-cli"
          ];
          meta = {
            description = "Headless kari node: serves the board of one host over HTTP on loopback";
            homepage = "https://github.com/lightheaded/kari";
            license = pkgs.lib.licenses.asl20;
            mainProgram = "kari-node";
          };
        };
      in
      {
        packages = {
          inherit kari-node;
          default = kari-node;
        };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            bun
            jq
          ];
        };
      }
    );
}
