"""Bundle the CI interpreter privately; do not require Python on the player's PC.

Avoids the PyInstaller/Unicorn CFG incompatibility reported in Unicorn #2281.
No Windows mitigation settings or executable flags are modified.
"""
import importlib.metadata
from pathlib import Path
import shutil
import sys
import zipfile


def build(output: Path, script: Path):
    if sys.platform != 'win32':
        raise RuntimeError('Windows Python bundle must be assembled on Windows')
    output.mkdir()
    prefix = Path(sys.base_prefix)
    version = f'{sys.version_info.major}{sys.version_info.minor}'
    shutil.copy2(prefix / 'python.exe', output / 'python.exe')
    for path in prefix.glob('*.dll'):
        shutil.copy2(path, output / path.name)
    shutil.copy2(prefix / 'LICENSE.txt', output / 'PYTHON-LICENSE.txt')
    shutil.copytree(prefix / 'DLLs', output / 'DLLs', ignore=shutil.ignore_patterns('*.pdb', '*.lib', '__pycache__'))
    # Package stdlib only, not the runner's pip, credentials, site packages or tests.
    with zipfile.ZipFile(output / f'python{version}.zip', 'w', zipfile.ZIP_DEFLATED) as archive:
        for path in sorted((prefix / 'Lib').rglob('*.py')):
            rel = path.relative_to(prefix / 'Lib')
            if any(part in ('site-packages', 'test', 'tests', '__pycache__', 'idlelib', 'tkinter', 'ensurepip') for part in rel.parts):
                continue
            archive.write(path, rel.as_posix())
    # ._pth disables environment/registry paths and user site customizations.
    (output / f'python{version}._pth').write_text(f'python{version}.zip\nDLLs\n.\n', encoding='utf-8')
    dist = importlib.metadata.distribution('unicorn')
    shutil.copytree(dist.locate_file('unicorn'), output / 'unicorn', ignore=shutil.ignore_patterns('__pycache__'))
    for entry in dist.files or []:
        if '.dist-info' in str(entry) and '..' not in entry.parts:
            dest = output / entry
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(dist.locate_file(entry), dest)
    shutil.copy2(script, output / 'continuum_reference_expander.py')
