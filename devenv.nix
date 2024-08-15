{ pkgs, lib, config, inputs, ... }:

{
  packages = with pkgs; [
    xorg.libX11
    xorg.libXi
    xorg.libXtst
    xorg.libxcb
    libxkbcommon
    xorg.libxkbfile
    cmake
    libclang
    pkg-config
  ];
  env.LIBCLANG_PATH="${pkgs.libclang.lib}/lib";
  enterShell = ''
    export BINDGEN_EXTRA_CLANG_ARGS="$NIX_CFLAGS_COMPILE \
      $(< ${pkgs.clang}/nix-support/libc-cflags) \
      $(< ${pkgs.clang}/nix-support/cc-cflags)"
  '';
  languages = {
    rust = {
      enable = true;
      channel = "stable";
      mold.enable = false;
      targets = [
        "aarch64-unknown-linux-gnu"
        "x86_64-unknown-linux-gnu"
        # "aarch64-pc-windows-msvc"
        # "aarch64-pc-windows-msvc"
        # "x86_64-apple-darwin"
        # "x86_64-apple-darwin"
      ];
    };
    javascript = {
      enable = true;
      pnpm.enable = true;
    };
  };
}
