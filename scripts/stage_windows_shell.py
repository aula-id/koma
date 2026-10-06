#!/usr/bin/env python3
"""Stage a Git for Windows usr/bin shell and generate its WiX fragment.

The release preparation script downloads the pinned PortableGit archive.
This script copies bash, coreutils, grep, sed, find, their MSYS DLLs, terminfo,
and upstream license text. It never executes an input binary.
"""
import argparse
import shutil
import uuid
from pathlib import Path
import xml.etree.ElementTree as ET

NS = 'http://schemas.microsoft.com/wix/2006/wi'
ET.register_namespace('', NS)

KEEP_PREFIXES = (
    'usr/bin/',
    'usr/share/licenses/',
    'usr/share/terminfo/',
    'etc/msystem.d/',
)
KEEP_FILES = {
    'LICENSE.txt',
    'etc/nsswitch.conf',
    'etc/msystem',
}


def node(parent, tag, **attrs):
    return ET.SubElement(parent, f'{{{NS}}}{tag}', attrs)


def wanted(relative: Path) -> bool:
    posix = relative.as_posix()
    if posix in KEEP_FILES:
        return True
    return any(posix.startswith(prefix) for prefix in KEEP_PREFIXES)


def stage(source: Path, destination: Path, fragment: Path) -> None:
    bash = source / 'usr' / 'bin' / 'bash.exe'
    if not bash.is_file():
        raise RuntimeError(f'Missing shell release input: {bash}')
    if destination.exists():
        shutil.rmtree(destination)
    destination.mkdir(parents=True)
    copied = 0
    for path in sorted(source.rglob('*')):
        if not path.is_file() or path.is_symlink():
            continue
        relative = path.relative_to(source)
        if not wanted(relative):
            continue
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, target)
        copied += 1
    required = [
        'usr/bin/bash.exe',
        'usr/bin/grep.exe',
        'usr/bin/ls.exe',
        'usr/bin/sed.exe',
        'usr/bin/find.exe',
        'usr/bin/msys-2.0.dll',
        'etc/nsswitch.conf',
        'LICENSE.txt',
    ]
    for relative in required:
        if not (destination / relative).is_file():
            raise RuntimeError(f'Staged shell is missing {relative}')
    files = sorted(p for p in destination.rglob('*') if p.is_file())
    root = ET.Element(f'{{{NS}}}Wix')
    fragment_node = node(root, 'Fragment')
    directory = node(
        node(fragment_node, 'DirectoryRef', Id='INSTALLDIR'),
        'Directory',
        Id='KomaShellDir',
        Name='shell',
    )
    group = node(fragment_node, 'ComponentGroup', Id='KomaShellFiles')
    directories = {Path('.'): directory}
    for path in files:
        relative = path.relative_to(destination)
        parent = Path('.')
        for part in relative.parts[:-1]:
            current = parent / part
            if current not in directories:
                identifier = 'ShDir_' + uuid.uuid5(uuid.NAMESPACE_URL, 'shell/' + current.as_posix()).hex
                directories[current] = node(directories[parent], 'Directory', Id=identifier, Name=part)
            parent = current
        guid = uuid.uuid5(uuid.NAMESPACE_URL, 'https://koma.run/shell/' + relative.as_posix())
        identifier = 'Sh_' + guid.hex
        component = node(directories[parent], 'Component', Id=identifier, Guid=str(guid), Win64='yes')
        node(component, 'File', Id=identifier + '_file', Source=str(path.resolve()), KeyPath='yes', Checksum='yes')
        node(group, 'ComponentRef', Id=identifier)
    ET.indent(root)
    fragment.parent.mkdir(parents=True, exist_ok=True)
    ET.ElementTree(root).write(fragment, encoding='utf-8', xml_declaration=True)
    print(f'Staged {copied} shell runtime files in {destination}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('source', type=Path)
    parser.add_argument('--destination', type=Path, default=Path('packaging/windows/shell'))
    parser.add_argument('--fragment', type=Path, default=Path('packaging/windows/shell-files.wxs'))
    args = parser.parse_args()
    stage(args.source, args.destination, args.fragment)
