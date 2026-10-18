#!/usr/bin/env python3
"""Create the single clickable Mac entry point beside an installation's support files."""
import argparse
from pathlib import Path
import plistlib
import shutil


def build(client: Path, output: Path):
    if output.exists():
        raise FileExistsError('App output already exists; preserve or back it up before replacing it')
    executable = output / 'Contents/MacOS'
    executable.mkdir(parents=True)
    shutil.copy2(client, executable / 'AIContinuum-bin')
    (executable / 'AIContinuum-bin').chmod(0o755)
    info = {
        'CFBundleExecutable': 'AIContinuum',
        'CFBundleIdentifier': 'org.aicontinuum.client',
        'CFBundleName': 'AIContinuum',
        'CFBundleDisplayName': 'AIContinuum',
        'CFBundlePackageType': 'APPL',
        'CFBundleShortVersionString': '0.1.0',
        'CFBundleVersion': '1',
        'NSHighResolutionCapable': True,
    }
    with (output / 'Contents/Info.plist').open('wb') as f:
        plistlib.dump(info, f)
    launcher = executable / 'AIContinuum'
    launcher.write_text('''#!/bin/bash
set -e
bin_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
install_dir=$(CDPATH= cd -- "$bin_dir/../../.." && pwd)
cd "$install_dir"
exec 2> >(tee -a "$install_dir/connection-diagnostics.log" >&2)
printf '\\n--- AIContinuum launch %s ---\\n' "$(date '+%Y-%m-%d %H:%M:%S')" >&2
export AICONTINUUM_TRACE=1
if [ -f "$install_dir/.continuum/expand" ]; then
    exec "$bin_dir/AIContinuum-bin" --assets "$install_dir" --continuum-expander "$install_dir/.continuum/expand" "$@"
fi
exec "$bin_dir/AIContinuum-bin" --assets "$install_dir" "$@"
''')
    launcher.chmod(0o755)
    (output / 'Contents/PkgInfo').write_bytes(b'APPL????')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--client', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    build(args.client, args.output)
