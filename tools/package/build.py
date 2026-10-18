#!/usr/bin/env python3
"""Build on the destination OS. Default archives contain no game assets or private image."""
import argparse
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile

REPO = Path(__file__).resolve().parents[2]
IMAGE_SHA256 = '551c950e264c752c5a0f49903cc9663d4f4c7b2b93a02ca7f34345bea77cad7b'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True, help='new output directory; never overwritten')
    parser.add_argument('--runtime-code', type=Path, help='optional private reference image; never used by public CI')
    args = parser.parse_args()
    output = args.output.resolve()
    if output.exists():
        parser.error('output already exists; choose a new directory')
    image = None
    if args.runtime_code:
        image = args.runtime_code.read_bytes()
        if hashlib.sha256(image).hexdigest() != IMAGE_SHA256:
            parser.error('runtime image is not the verified reference build')
    subprocess.run(['cargo', 'build', '--locked', '--release'], cwd=REPO, check=True)
    suffix = '.exe' if os.name == 'nt' else ''
    output.mkdir(parents=True)
    shutil.copy2(REPO / 'target/release' / ('AIContinuum' + suffix), output)
    with tempfile.TemporaryDirectory(prefix='aic-package-') as temp:
        temp = Path(temp)
        subprocess.run([sys.executable, '-m', 'PyInstaller', '--noconfirm', '--clean', '--onedir',
            '--name', 'expand', '--noupx', '--collect-all', 'unicorn', '--copy-metadata', 'unicorn',
            '--distpath', str(temp / 'dist'), '--workpath', str(temp / 'build'), '--specpath', str(temp),
            str(REPO / 'tools/protocol/continuum_reference_expander.py')], check=True)
        shutil.copytree(temp / 'dist/expand', output / '.continuum', symlinks=True)
    # Explicitly include helper source and upstream license metadata.
    source = output / '.continuum/source'
    source.mkdir()
    shutil.copy2(REPO / 'tools/protocol/continuum_reference_expander.py', source)
    for package in ['unicorn', 'pyinstaller']:
        dist = importlib.metadata.distribution(package)
        for file in dist.files or []:
            if any('license' in part.lower() or 'copying' in part.lower() for part in file.parts):
                path = dist.locate_file(file)
                if path.is_file():
                    dst = source / package / file
                    dst.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(path, dst)
    if image is not None:
        (output / '.continuum/runtime-code.bin').write_bytes(image)
    # A deliberately missing image must fail clearly, not crash due to missing Python/Unicorn.
    env = dict(os.environ)
    env['AICONTINUUM_RUNTIME_CODE'] = str(output / '.continuum/nonexistent-test-image.bin')
    smoke = subprocess.run([str(output / '.continuum' / ('expand' + suffix)), '123'],
        capture_output=True, env=env, timeout=15)
    if smoke.returncode != 1 or b'Continuum reference expansion failed:' not in smoke.stderr:
        raise RuntimeError('packaged helper failed its dependency/diagnostic smoke test')
    shutil.copy2(REPO / 'docs/PLATFORMS.md', output / 'START-HERE.md')
    if sys.platform == 'darwin':
        from macos_app import build as build_app
        build_app(output / 'AIContinuum', output / 'AIContinuum.app')
        (output / 'AIContinuum').unlink()
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip()
    manifest = {'commit': commit, 'os': platform.system(), 'architecture': platform.machine(),
        'dirty_source': bool(subprocess.check_output(['git', 'status', '--porcelain', '--untracked-files=no'], cwd=REPO)),
        'private_runtime_included': image is not None, 'assets_included': False,
        'sha256': {str(p.relative_to(output)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in output.rglob('*') if p.is_file()}}
    (output / 'package.json').write_text(json.dumps(manifest, indent=2) + '\n')
    archive_format = 'zip' if os.name == 'nt' else 'gztar'
    archive = shutil.make_archive(str(output), archive_format, root_dir=output.parent, base_dir=output.name)
    print('Created ' + archive)


if __name__ == '__main__':
    main()
