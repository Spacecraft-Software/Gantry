# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
#
# Gantry package, callPackage-style. Built from the local tree; a release
# build would switch `src` to a fetchFromGitHub pinned by tag and hash.
{
  lib,
  rustPlatform,
  testers,
}:

rustPlatform.buildRustPackage (finalAttrs: {
  pname = "gantry";
  version = "0.1.0";

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src
      ../assets
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;

  passthru.tests.version = testers.testVersion {
    package = finalAttrs.finalPackage;
    command = "gantry --version";
  };

  meta = {
    description = "Themed, animated greetd greeter for the Linux console and kiosk terminals";
    homepage = "https://github.com/Spacecraft-Software/Gantry";
    license = lib.licenses.gpl3Plus;
    mainProgram = "gantry";
    platforms = lib.platforms.linux;
  };
})
