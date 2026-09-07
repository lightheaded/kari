{
  description = "kari — a Kanban board for Claude Code sessions; this flake builds the headless node and the server";

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
  # A node installed this way is pinned by the flake, so leave `--auto-update`
  # off: it needs to write over its own binary, and this one lives in the
  # read-only store. Update the host instead. `--auto-update` is for a node
  # installed as a plain file, and it also needs `Restart = "always"` — it
  # stops cleanly once it has written the new binary, and `on-failure` would
  # read that as the end and leave the host with no node.
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
        # The server is the one part of kari with nothing on its host: no
        # Claude Code, no login, no transcript to read. So it is the part that
        # suits a container. The node does not — it exists to watch the
        # sessions of a real user on a real machine.
        kari-server-image = pkgs.dockerTools.buildLayeredImage {
          name = "kari-server";
          tag = manifest.workspace.package.version;
          # A writable /data for the token, and nothing else. No shell, no
          # package manager: the image holds one static-ish binary and its libc.
          extraCommands = "mkdir -p data";
          config = {
            Entrypoint = [ "${kari-node}/bin/kari-server" ];
            Cmd = [ "serve" ];
            ExposedPorts = { "47312/tcp" = { }; };
            Env = [
              "HOME=/data"
              "XDG_CONFIG_HOME=/data/.config"
            ];
            Volumes = { "/data" = { }; };
          };
        };
      in
      {
        packages = {
          inherit kari-node;
          default = kari-node;
        }
        # dockerTools builds Linux images; asking for one on darwin is an error
        # rather than an empty answer, so the attribute is simply absent there.
        // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux { inherit kari-server-image; };

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
