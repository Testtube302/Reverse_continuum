# Windows and Linux preview builds

This is a native client with one executable per platform. The encryption helper
is bundled under `.continuum`; users do not need Python installed. Continuum,
VIE and plaintext transport selection uses the same launcher. These are preview
packages; a passing build is not proof of public-server acceptance or GUI/audio
compatibility on every device.

## Install and run

1. Extract the complete package, including `.continuum`. On Windows run
   `AIContinuum.exe`; on Linux run `./AIContinuum` in a graphical desktop session.
2. Supply your Continuum assets beside the executable: `graphics/`, `sound/`,
   `menures.dll`, `menu040.dll`, and any desired `skins/` and help/data files.
   Alternatively pass `--assets /path/to/Continuum`. No original game assets are
   present in the public build artifacts. DLLs are read as resource data, not loaded.
3. For Continuum encryption, supply the already-verified private `runtime-code.bin`
   under `.continuum/`. The public artifacts intentionally do not contain it.
   Keep the helper directory with the client when moving the installation.
4. Choose a zone in the launcher. There is no encrypted/unencrypted launcher choice.

Linux builds target x86-64 Ubuntu 22.04 or compatible newer systems with X11/OpenGL
and ALSA runtime libraries. Wayland desktops need XWayland. Windows CI builds target
64-bit Windows with the MSVC toolchain. Native GUI/audio/gameplay acceptance still
needs testing on the destination desktop; CI only covers headless tests and helper
startup, without assets or the private runtime image.

Settings and downloaded maps live in `%LOCALAPPDATA%\AIContinuum` on Windows and
`$XDG_DATA_HOME/aicontinuum` (default `~/.local/share/aicontinuum`) on Linux. Existing
settings supplied in the assets directory are copied only if no saved copy exists.
`AICONTINUUM_DATA` overrides the data directory. A `.portable` marker in the assets
directory keeps settings there instead. Existing macOS installations retain their
current portable behavior. Linux asset lookup tolerates Windows filename casing.
Never include your personal profile.dat or connection logs in a shared package.

## Build on the destination OS

Install Rust and Python 3.12. Linux build dependencies: `libasound2-dev`,
`libx11-dev`, `libxi-dev`, `libgl1-mesa-dev`, and the standard C compiler/toolchain.
Use the matching OS to freeze the helper; PyInstaller is not a cross compiler.

```
python -m venv .package-venv
# Windows: .package-venv\Scripts\activate
# Linux/macOS: source .package-venv/bin/activate
python -m pip install -r tools/package/requirements.txt
cargo test --locked -- --test-threads=1
python tools/package/build.py --output target/packages/AIContinuum-preview
```

The output directory must not already exist. The package script builds the client,
freezes the helper including Python/Unicorn, checks its missing-image diagnostic,
includes helper source/license metadata, and writes hashes and a zip/tar.gz archive.
The optional `--runtime-code /private/path/runtime-code.bin` adds the verified image
only to a private local package. It is not used by GitHub Actions. Game assets and
saved accounts are never copied by this script. Keep private packages out of Git.

The CI recipe at `tools/package/platforms.yml` is ready to install as
`.github/workflows/platforms.yml`. Publishing that active workflow requires GitHub
`workflow` permission; it has not run yet. Once activated, it builds/tests Windows
and Linux on native runners. Its artifacts contain executables/support libraries,
never the private reference image or game assets.

Current evidence: macOS regression tests, release build, packaged-helper equivalence
and local encrypted arena entry pass. Windows GNU release build and test compilation
pass. Linux x86-64 code/test cross-check passes. Native Windows/Linux execution and
complete destination-platform packages remain pending.

## Identity mapping and validation limits

Windows uses GetUserNameA/GetComputerNameA string CRCs, the Windows system-volume
serial, the current timezone bias, and BCryptGenRandom for fresh nonces. Linux uses
getpwuid_r/gethostname, a namespaced mapping of the system machine ID, local timezone,
and /dev/urandom. Existing macOS collection is preserved. Unresolved legacy metadata
fields remain zero; this is not a claim of full original-client metadata parity.
No captured identity is replayed and identifiers are not written to diagnostics.

Before calling a package production-ready, test launcher/settings persistence,
sound, key bindings, all three transport choices, private/team chat, arena changes,
ship movement, spectator mode, security challenges and reconnect behavior on each OS.
Use a local server and dummy accounts first. Public-server acceptance requires a
separate authorized test on the intended machine/network.

API/build references:
- [Windows volume information](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getvolumeinformationw)
- [Windows timezone information](https://learn.microsoft.com/en-us/windows/win32/api/timezoneapi/nf-timezoneapi-gettimezoneinformation)
- [Windows system RNG](https://learn.microsoft.com/en-us/windows/win32/api/bcrypt/nf-bcrypt-bcryptgenrandom)
- [Windows computer name](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-getcomputernamea)
- [PyInstaller packaging](https://pyinstaller.org/en/stable/usage.html)
