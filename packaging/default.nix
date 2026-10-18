# SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Nix package for Vernier. From a flake (e.g. Steelbore OS Bravais):
#   vernier = pkgs.callPackage ./packaging/default.nix { };
# or directly:
#   nix-build -E 'with import <nixpkgs> {}; callPackage ./packaging/default.nix {}'
{
  lib,
  rustPlatform,
  makeWrapper,
  wayland,
  libxkbcommon,
  vulkan-loader,
  libGL,
}:

rustPlatform.buildRustPackage {
  pname = "vernier";
  version = "0.1.0";

  # Skip local build output so a prior `cargo build` does not bloat the source.
  src = lib.cleanSourceWith {
    src = lib.cleanSource ../.;
    filter = path: type: !(type == "directory" && baseNameOf path == "target");
  };
  cargoLock.lockFile = ../Cargo.lock;

  nativeBuildInputs = [ makeWrapper ];

  # The binary links only libc; iced (winit, wgpu) opens these at run time.
  postFixup = ''
    wrapProgram $out/bin/vnr \
      --prefix LD_LIBRARY_PATH : ${
        lib.makeLibraryPath [
          wayland
          libxkbcommon
          vulkan-loader
          libGL
        ]
      }
  '';

  meta = {
    description = "Arrange, rotate, scale and mirror displays on Wayland and X11";
    homepage = "https://Vernier.SpacecraftSoftware.org/";
    license = lib.licenses.gpl3Plus;
    mainProgram = "vnr";
    platforms = lib.platforms.linux;
  };
}
