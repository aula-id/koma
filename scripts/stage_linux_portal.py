#!/usr/bin/env python3
"""Stage the minimal one-frame GStreamer/PipeWire runtime for desktop AppImages.

Run on the native Ubuntu packaging runner after installing the named packages.
No portal session is opened and no desktop is captured during staging.
"""
from pathlib import Path
import re
import shutil
import subprocess

ROOT = Path('packaging/linux/gstreamer')
SYSTEM = {'libc.so.6', 'libm.so.6', 'libpthread.so.0', 'libdl.so.2', 'librt.so.1', 'libutil.so.1', 'libgcc_s.so.1'}
PACKAGES = {'gstreamer1.0-tools', 'gstreamer1.0-plugins-base', 'gstreamer1.0-plugins-good', 'gstreamer1.0-pipewire', 'libgstreamer1.0-0', 'pipewire-bin', 'libpipewire-0.3-modules', 'libspa-0.2-modules'}
seen = set()

def copy(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source.resolve(), destination)

def libraries(binary: Path) -> None:
    if binary.resolve() in seen:
        return
    seen.add(binary.resolve())
    result = subprocess.run(['ldd', str(binary)], check=True, capture_output=True, text=True)
    if 'not found' in result.stdout:
        raise RuntimeError(f'Missing dependency for {binary}: {result.stdout}')
    for line in result.stdout.splitlines():
        match = re.match(r'\s*(\S+) => (/\S+)', line)
        if not match or match[1] in SYSTEM:
            continue
        source = Path(match[2])
        copy(source, ROOT / 'lib' / match[1])
        libraries(source)
    owner = subprocess.run(['dpkg-query', '-S', str(binary.resolve())], capture_output=True, text=True)
    if owner.returncode == 0:
        PACKAGES.add(owner.stdout.split(': ', 1)[0].split(':', 1)[0])

def package_files(package: str) -> list[Path]:
    return [Path(p) for p in subprocess.check_output(['dpkg-query', '-L', package], text=True).splitlines()]

def pipewire_runtime() -> None:
    # PipeWire loads these modules with dlopen, so ldd on pipewiresrc is insufficient.
    config = next((p for p in [Path('/usr/share/pipewire/client.conf'), Path('/etc/pipewire/client.conf')] if p.is_file()), None)
    if config is None:
        raise RuntimeError('PipeWire client.conf missing; install pipewire-bin')
    copy(config, ROOT / 'share/pipewire/client.conf')
    lines = '\n'.join(line.split('#', 1)[0] for line in config.read_text().splitlines())
    modules = package_files('libpipewire-0.3-modules')
    spa = package_files('libspa-0.2-modules')
    for name in set(re.findall(r'\bname\s*=\s*(libpipewire-module-[\w-]+)', lines)):
        source = next((p for p in modules if p.name == name + '.so'), None)
        if source is None:
            raise RuntimeError(f'PipeWire client module missing: {name}')
        copy(source, ROOT / 'lib/pipewire-0.3' / source.name)
        libraries(source)
    names = set(re.findall(r'=\s*([\w-]+/libspa-[\w-]+)', lines))
    names.add('support/libspa-support')
    for name in names:
        source = next((p for p in spa if p.as_posix().endswith('/' + name + '.so')), None)
        if source is None:
            raise RuntimeError(f'PipeWire client SPA module missing: {name}')
        copy(source, ROOT / 'lib/spa-0.2' / (name + '.so'))
        libraries(source)

def main() -> None:
    launch = shutil.which('gst-launch-1.0')
    if not launch:
        raise RuntimeError('gstreamer1.0-tools is required')
    plugin_dir = Path(subprocess.check_output(['pkg-config', '--variable=pluginsdir', 'gstreamer-1.0'], text=True).strip())
    for choices in [('libgstcoreelements.so',), ('libgstpipewire.so',), ('libgstpng.so',), ('libgstvideoconvertscale.so', 'libgstvideoconvert.so')]:
        plugin = next((plugin_dir / name for name in choices if (plugin_dir / name).is_file()), None)
        if plugin is None:
            raise RuntimeError(f'Missing GStreamer plugin: {choices}')
        copy(plugin, ROOT / 'lib/gstreamer-1.0' / plugin.name)
        libraries(plugin)
    for tool in ['gst-launch-1.0', 'gst-inspect-1.0']:
        binary = shutil.which(tool)
        if not binary:
            raise RuntimeError(f'Missing GStreamer tool: {tool}')
        copy(Path(binary), ROOT / 'bin' / tool)
        libraries(Path(binary))
    scanner = next((p for p in package_files('libgstreamer1.0-0') if p.name == 'gst-plugin-scanner'), None)
    if scanner is None:
        raise RuntimeError('GStreamer plugin scanner missing')
    copy(scanner, ROOT / 'libexec/gstreamer-1.0/gst-plugin-scanner')
    libraries(scanner)
    pipewire_runtime()
    for package in sorted(PACKAGES):
        notice = Path('/usr/share/doc') / package / 'copyright'
        if notice.is_file():
            copy(notice, ROOT / 'share/doc' / package / 'copyright')
    print(f'Staged portal capture runtime and dependencies under {ROOT}')

if __name__ == '__main__':
    main()
