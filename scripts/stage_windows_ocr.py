#!/usr/bin/env python3
"""Stage an installed Tesseract runtime and generate its existing-WiX fragment.

Only the release preparation script downloads/installs the pinned upstream asset.
This script copies runtime files/data/notices, never executes an input binary.
"""
import argparse
import shutil
import uuid
from pathlib import Path
import xml.etree.ElementTree as ET

NS = 'http://schemas.microsoft.com/wix/2006/wi'
ET.register_namespace('', NS)

def node(parent, tag, **attrs):
    return ET.SubElement(parent, f'{{{NS}}}{tag}', attrs)

def stage(source: Path, destination: Path, fragment: Path) -> None:
    required = ['tesseract.exe', 'tessdata/eng.traineddata']
    for relative in required:
        if not (source / relative).is_file():
            raise RuntimeError(f'Missing OCR release input: {source / relative}')
    destination.mkdir(parents=True, exist_ok=True)
    for path in sorted(source.rglob('*')):
        if not path.is_file() or path.is_symlink():
            continue
        relative = path.relative_to(source)
        # Include all DLLs, English data, configs and upstream license/notices;
        # omit uninstallers/training executables and other language models.
        if path.suffix.lower() == '.exe' and relative.as_posix() != 'tesseract.exe':
            continue
        if path.suffix.lower() == '.traineddata' and path.name != 'eng.traineddata':
            continue
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, target)
    files = sorted(p for p in destination.rglob('*') if p.is_file())
    if not any(p.suffix.lower() == '.dll' for p in files):
        raise RuntimeError('Tesseract runtime DLLs were not staged')
    root = ET.Element(f'{{{NS}}}Wix')
    fragment_node = node(root, 'Fragment')
    directory = node(node(fragment_node, 'DirectoryRef', Id='INSTALLDIR'), 'Directory', Id='KomaOcrDir', Name='ocr')
    group = node(fragment_node, 'ComponentGroup', Id='KomaOcrFiles')
    directories = {Path('.'): directory}
    for path in files:
        relative = path.relative_to(destination)
        parent = Path('.')
        for part in relative.parts[:-1]:
            current = parent / part
            if current not in directories:
                identifier = 'OcrDir_' + uuid.uuid5(uuid.NAMESPACE_URL, current.as_posix()).hex
                directories[current] = node(directories[parent], 'Directory', Id=identifier, Name=part)
            parent = current
        guid = uuid.uuid5(uuid.NAMESPACE_URL, 'https://koma.run/ocr/' + relative.as_posix())
        identifier = 'Ocr_' + guid.hex
        component = node(directories[parent], 'Component', Id=identifier, Guid=str(guid), Win64='yes')
        node(component, 'File', Id=identifier + '_file', Source=str(path.resolve()), KeyPath='yes', Checksum='yes')
        node(group, 'ComponentRef', Id=identifier)
    ET.indent(root)
    fragment.parent.mkdir(parents=True, exist_ok=True)
    ET.ElementTree(root).write(fragment, encoding='utf-8', xml_declaration=True)
    print(f'Staged {len(files)} OCR runtime files in {destination}')

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('source', type=Path)
    parser.add_argument('--destination', type=Path, default=Path('packaging/windows/ocr'))
    parser.add_argument('--fragment', type=Path, default=Path('packaging/windows/ocr-files.wxs'))
    args = parser.parse_args()
    stage(args.source, args.destination, args.fragment)
